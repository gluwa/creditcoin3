use crate::Error;
use arc_swap::access::Access;

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
    pub async fn new(client: &'a mut crate::Client) -> Result<Self, Error> {
        let runtime_api = loop {
            let reconnect = match client
                .api()
                .load()
                .runtime_api()
                .at_latest()
                .await
                .map_err(Into::<Error>::into)
            {
                Ok(runtime_api) => break runtime_api,
                Err(Error::ConnectionError(reconnect)) => reconnect,
                Err(err) => return Err(err),
            };

            client.reconnect(reconnect).await;
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
    ) -> Result<Call::ReturnType, Error> {
        loop {
            match self
                .runtime_api
                .call(call())
                .await
                .map_err(Into::<Error>::into)
            {
                Ok(res) => break Ok(res),
                Err(Error::ConnectionError(reconnect)) => {
                    self.client.reconnect(reconnect).await;

                    let runtime_api = match self.client.api().load().runtime_api().at_latest().await
                    {
                        Ok(runtime_api) => runtime_api,
                        Err(err) => {
                            tracing::warn!(?err, "Failed to re-connect to CC3 runtime api...");
                            continue;
                        }
                    };

                    self.runtime_api = runtime_api;
                    self.client.reset_connection_delay();
                }
                Err(err) => return Err(err),
            }
        }
    }
}
