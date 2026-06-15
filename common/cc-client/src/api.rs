use crate::Error;
use arc_swap::access::Access;

/// Auto-reconnecting substrate [`RuntimeApi`].
///
/// Wraps a runtime API call in a reconnect loop: on `Error::ConnectionError`, calls
/// [`crate::Client::reconnect`] (which is itself bounded + cancellable via Ctrl-C) and
/// retries. The cached `runtime_api` is invalidated on every reconnect and lazily
/// refreshed on the next iteration — that way we never call against a stale handle
/// (which would either reach the dead pre-reconnect `RpcClient` or, with an unreliable
/// classifier, return stale data against the OLD block hash on a still-alive OLD WS).
/// Refresh failures stay inside the retry loop: a transient `at_latest` error gets
/// re-classified and the next iteration either succeeds or reconnects again.
///
/// [`RuntimeApi`]: subxt::runtime_api::RuntimeApi
pub struct ReconnectingRuntimeApi<'a> {
    client: &'a crate::Client,
    runtime_api: Option<
        subxt::runtime_api::RuntimeApi<
            subxt::SubstrateConfig,
            subxt::OnlineClient<subxt::SubstrateConfig>,
        >,
    >,
}

impl<'a> ReconnectingRuntimeApi<'a> {
    /// Build a fresh handle. Tries `at_latest`, reconnects on `ConnectionError`, surfaces
    /// any other failure to the caller.
    pub async fn new(client: &'a crate::Client) -> Result<Self, Error> {
        let mut err = Ok(());
        let runtime_api = loop {
            match std::mem::replace(&mut err, Ok(())) {
                Err(Error::ConnectionError(reconnect)) => {
                    if let Err(err_new) = client.reconnect(reconnect).await {
                        err = Err(err_new);
                        continue;
                    }
                }
                Err(err) => return Err(err),
                Ok(()) => {}
            }

            match client
                .api()
                .load()
                .runtime_api()
                .at_latest()
                .await
                .map_err(Into::<Error>::into)
            {
                Ok(runtime_api) => break runtime_api,
                Err(err_new) => err = Err(err_new),
            }
        };

        client.reset_connection_delay();

        Ok(Self {
            client,
            runtime_api: Some(runtime_api),
        })
    }

    /// Run `call()` against the cached `RuntimeApi`. On connection errors, reconnects,
    /// invalidates the cached handle, and lazily refreshes it on the next loop pass.
    /// Unbounded retry is intentional — long RPC outages should be ridden out rather
    /// than crashing the caller's task; shutdown propagates via the caller dropping the
    /// future (the awaits inside `Client::reconnect`'s backoff are cancellation points).
    pub async fn call<Call: subxt::runtime_api::Payload>(
        &mut self,
        call: impl Fn() -> Call,
    ) -> Result<Call::ReturnType, Error> {
        let mut err = Ok(());
        loop {
            match std::mem::replace(&mut err, Ok(())) {
                Err(Error::ConnectionError(reconnect)) => {
                    err = self.client.reconnect(reconnect).await;
                    continue;
                }
                Err(err) => return Err(err),
                Ok(()) => {}
            }

            // Ensure we have a fresh `RuntimeApi`. The cache is invalidated to `None`
            // after every reconnect (below), so this branch runs once per recovery
            // cycle and re-attempts `at_latest` until it succeeds.
            let runtime_api = match self.runtime_api.as_ref() {
                Some(api) => api,
                None => {
                    match self
                        .client
                        .api()
                        .load()
                        .runtime_api()
                        .at_latest()
                        .await
                        .map_err(Into::<Error>::into)
                    {
                        Ok(api) => self.runtime_api.insert(api),
                        Err(err_new) => {
                            err = Err(err_new);
                            continue;
                        }
                    }
                }
            };

            match runtime_api.call(call()).await.map_err(Into::<Error>::into) {
                Ok(res) => {
                    self.client.reset_connection_delay();
                    return Ok(res);
                }
                Err(err_new) => {
                    err = Err(err_new);
                    // Crucially, invalidate the cached handle. Without this, the next
                    // iteration would call against the pre-reconnect RuntimeApi —
                    // either dead (re-classifies to ConnectionError and loops, the
                    // self-correcting path) or, worse, alive against an old block
                    // hash on a stale-but-functional WS (silently returning stale
                    // data; the failure mode the bot flagged).
                    self.runtime_api = None;
                }
            }
        }
    }
}
