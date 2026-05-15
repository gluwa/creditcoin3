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
    runtime_api: subxt::runtime_api::RuntimeApi<
        subxt::SubstrateConfig,
        subxt::OnlineClient<subxt::SubstrateConfig>,
    >,
}

impl<'a> ReconnectingRuntimeApi<'a> {
    pub async fn new(client: &'a mut crate::Client) -> Result<Self, Interrupt<crate::Error>> {
        let runtime_api = match client.api().load().runtime_api().at_latest().await {
            Ok(runtime_api) => runtime_api,
            Err(err) => reconnect(client, err).await?,
        };

        Ok(Self {
            client,
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
                Err(err) => {
                    self.runtime_api = reconnect(self.client, err).await?;
                }
            }
        }
    }
}

async fn reconnect(
    client: &crate::Client,
    err: subxt::Error,
) -> Result<
    subxt::runtime_api::RuntimeApi<
        subxt::SubstrateConfig,
        subxt::OnlineClient<subxt::SubstrateConfig>,
    >,
    Interrupt<crate::Error>,
> {
    tracing::warn!(?err, "CC3 connection lost");

    let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
        .max_delay(std::time::Duration::from_millis(5_000))
        .map(tokio_retry::strategy::jitter);
    let reconnect = || {
        tracing::warn!("Reconnecting to CC3...");

        async move {
            // `Client::reconnect` now takes `&self` and atomically swaps the
            // underlying subxt connection for every `Arc<Client>` (and shared
            // borrow) holder. We can therefore refresh in place rather than
            // shuffling a value-cloned `Client` around.
            client.reconnect().await.map_err(|err| {
                tracing::error!(?err, "Failed to reconnect to CC3");
                err
            })?;

            let runtime_api = client
                .api()
                .load()
                .runtime_api()
                .at_latest()
                .await
                .map_err(|err| {
                    tracing::error!(?err, "Failed to reconnect to CC3");
                    crate::Error::SubxtError(err)
                })?;

            Ok::<_, crate::Error>(runtime_api)
        }
    };
    tokio::select! {
        retry = tokio_retry::Retry::spawn(strategy, reconnect) => {
            Ok(retry.expect("Unbounded retry cannot error"))
        }
        _ = tokio::signal::ctrl_c() => {
            Err(Interrupt::Stop)
        }
    }
}
