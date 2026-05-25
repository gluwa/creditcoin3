use crate::Error;
use arc_swap::access::Access;

/// Auto-reconnecting substrate [`RuntimeApi`].
///
/// Wraps a runtime API call in a bounded reconnect loop: on `Error::ConnectionError`, calls
/// [`crate::Client::reconnect`] (which is itself bounded by `MAX_RECONNECT_ATTEMPTS` and
/// cancellable via Ctrl-C) and retries. If `reconnect()` returns `Err(_)` — exhaustion or
/// shutdown — the error is surfaced to the caller so the task can crash cleanly instead
/// of spinning.
///
/// [`RuntimeApi`]: subxt::runtime_api::RuntimeApi
pub struct ReconnectingRuntimeApi<'a> {
    client: &'a crate::Client,
    runtime_api: subxt::runtime_api::RuntimeApi<
        subxt::SubstrateConfig,
        subxt::OnlineClient<subxt::SubstrateConfig>,
    >,
}

impl<'a> ReconnectingRuntimeApi<'a> {
    pub async fn new(client: &'a crate::Client) -> Result<Self, Error> {
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

            // Propagate reconnect exhaustion / shutdown up: callers shouldn't loop forever
            // hoping the RPC eventually answers.
            client.reconnect(reconnect).await?;
        };
        client.reset_connection_delay();

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
                Ok(res) => {
                    self.client.reset_connection_delay();
                    break Ok(res);
                }
                Err(Error::ConnectionError(reconnect)) => {
                    self.client.reconnect(reconnect).await?;

                    let runtime_api = match self.client.api().load().runtime_api().at_latest().await
                    {
                        Ok(runtime_api) => runtime_api,
                        Err(err) => {
                            tracing::warn!(?err, "Failed to re-connect to CC3 runtime api...");
                            continue;
                        }
                    };

                    self.runtime_api = runtime_api;
                }
                Err(err) => return Err(err),
            }
        }
    }
}
