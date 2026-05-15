use arc_swap::access::Access;
use user::prelude::*;

/// Auto-reconnecting substrate [`RuntimeApi`].
///
/// Will attempt to restore connection on any failed runtime call. Reconnection attempts are
/// unbounded and can only be aborted via manual user cancellation (Ctrl-C).
///
/// Holds a borrow of the underlying [`crate::Client`] (taken as `&mut` for
/// historical reasons — every consumer here calls through a mutable handle).
/// The underlying [`crate::Client::reconnect`] now uses interior mutability,
/// so the borrow is `&Client` for reconnect purposes; we keep the `&mut`
/// receiver to avoid churn at every call site.
///
/// [`RuntimeApi`]: subxt::runtime_api::RuntimeApi
pub struct ReconnectingRuntimeApi<'a> {
    client: &'a mut crate::Client,
    delay: tokio_retry::strategy::ExponentialBackoff,
    runtime_api: subxt::runtime_api::RuntimeApi<
        subxt::SubstrateConfig,
        subxt::OnlineClient<subxt::SubstrateConfig>,
    >,
}

impl<'a> ReconnectingRuntimeApi<'a> {
    pub async fn new(client: &'a mut crate::Client) -> Result<Self, Interrupt<crate::Error>> {
        let mut delay = util::exponential_retry_delay();

        let runtime_api = loop {
            match client.api().load().runtime_api().at_latest().await {
                Ok(runtime_api) => break runtime_api,
                Err(err) => tracing::warn!(?err, "CC3 connection lost..."),
            }

            let delay = delay
                .next()
                .map(tokio_retry::strategy::jitter)
                .unwrap_or(util::MAX_DELAY);
            tokio::time::sleep(delay).await;

            if let Err(err) = client.reconnect().await {
                tracing::warn!(?err, "Failed to reconnect to CC3");
            }
        };

        delay = util::exponential_retry_delay();

        Ok(Self {
            client,
            delay,
            runtime_api,
        })
    }

    pub async fn call<Call: subxt::runtime_api::Payload>(
        &mut self,
        call: impl Fn() -> Call,
    ) -> Result<Call::ReturnType, Interrupt<crate::Error>> {
        loop {
            match self.runtime_api.call(call()).await {
                Ok(res) => break Ok(res),
                Err(err) => self.reconnect(err).await,
            }
        }
    }

    async fn reconnect(&mut self, err: subxt::Error) {
        let runtime_api = loop {
            tracing::warn!(?err, "CC3 connection lost...");

            let delay = self
                .delay
                .next()
                .map(tokio_retry::strategy::jitter)
                .unwrap_or(util::MAX_DELAY);
            tokio::time::sleep(delay).await;

            if let Err(err) = self.client.reconnect().await {
                tracing::warn!(?err, "Failed to reconnect to CC3");
            }

            match self.client.api().load().runtime_api().at_latest().await {
                Ok(runtime_api) => break runtime_api,
                Err(err) => tracing::warn!(?err, "CC3 connection lost..."),
            }
        };

        self.delay = util::exponential_retry_delay();
        self.runtime_api = runtime_api;
    }
}

mod util {
    pub const MAX_DELAY: std::time::Duration = std::time::Duration::from_millis(5_000);

    pub fn exponential_retry_delay() -> tokio_retry::strategy::ExponentialBackoff {
        tokio_retry::strategy::ExponentialBackoff::from_millis(100).max_delay(MAX_DELAY)
    }
}
