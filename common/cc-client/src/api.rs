use user::prelude::*;

/// Auto-reconnecting substrate [`RuntimeApi`].
///
/// Restores the connection on *transient* runtime-call failures (ws drop, restart-needed,
/// request timeout, server-side "unavailable / shutting down" replies — see
/// [`crate::Error::from`]). Reconnection attempts are unbounded and can only be aborted via
/// manual user cancellation (Ctrl-C). A *deterministic* failure (decode error, metadata drift,
/// a structurally invalid call) is surfaced to the caller instead of reconnected — reconnecting
/// would re-issue the same call against a healthy connection and loop forever without ever
/// returning an error.
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
        let runtime_api = match client.api().runtime_api().at_latest().await {
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
                // Classify before reacting. `crate::Error::from` runs the shared transient
                // classifier: only a recoverable disconnect becomes `ConnectionError`.
                Err(err) => match crate::Error::from(err) {
                    crate::Error::ConnectionError(crate::Reconnect(err)) => {
                        self.runtime_api = reconnect(self.client, err).await?;
                    }
                    // Deterministic fault — reconnecting can't fix it and would hot-loop the
                    // same failure. Surface it so the caller decides.
                    fatal => break Err(Interrupt::Cont(fatal)),
                },
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

    // No retry strategy stacked on top here: `Client::reconnect` performs a single dial per
    // call but paces itself via the client-wide shared `BackoffState` (it sleeps until the
    // shared `next_attempt_at` before dialing, and every failure advances it exponentially).
    // A plain loop is therefore already backoff-limited — wrapping it in `tokio_retry` would
    // just stack a second, uncoordinated backoff on top of the shared one.
    let reconnect = async {
        loop {
            tracing::warn!("Reconnecting to CC3...");
            // `Client::reconnect` takes `&self` and atomically swaps the underlying subxt
            // connection for every `Arc<Client>` (and shared borrow) holder.
            if let Err(err) = client.reconnect().await {
                tracing::error!(?err, "Failed to reconnect to CC3");
                continue;
            }
            match client.api().runtime_api().at_latest().await {
                Ok(runtime_api) => break runtime_api,
                Err(err) => {
                    // Dial succeeded but the fresh connection failed immediately. The shared
                    // backoff was just reset by the successful dial, so pause briefly to avoid
                    // a hot connect/fail loop against a node that accepts sockets but can't
                    // serve requests yet.
                    tracing::error!(?err, "Reconnected but at_latest failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    };
    tokio::select! {
        runtime_api = reconnect => Ok(runtime_api),
        _ = tokio::signal::ctrl_c() => Err(Interrupt::Stop),
    }
}
