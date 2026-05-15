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
        let runtime_api = loop {
            let err = match client.api().load().runtime_api().at_latest().await {
                Ok(runtime_api) => break runtime_api,
                Err(err) => err,
            };

            client.reconnect(err).await;
            client.reset_connection_delay();
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
                Err(err) => self.reconnect(err).await,
            }
        }
    }

    async fn reconnect(&mut self, err: subxt::Error) {
        loop {
            self.client.reconnect(&err).await;

            match self.client.api().load().runtime_api().at_latest().await {
                Ok(runtime_api) => {
                    self.runtime_api = runtime_api;
                    break;
                }
                Err(err) => tracing::warn!(?err, "Failed to re-connect to CC3 runtime api..."),
            }
        }

        self.client.reset_connection_delay();
    }
}
