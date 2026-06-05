//! Retry shim for runtime API calls.
//!
//! Background:
//!   v1 wrapped every `runtime_api.call(...)` in `cc_client::api::ReconnectingRuntimeApi`,
//!   which takes `&mut Client` and transparently retries on transient WS disconnects, calling
//!   `Client::reconnect()` between attempts. v2 holds `Arc<Client>` (one swap shared across all
//!   tasks) so we can't get `&mut`; we need a `&Arc<Client>`-friendly equivalent.
//!
//! `with_retries` retries an async closure with exponential backoff. If the inner error matches
//! [`is_transient`] — JSON-RPC disconnects, subxt RPC errors, generic IO errors — it also
//! triggers `cc3.reconnect()` between attempts. Because `cc3` is shared across all tasks via
//! one `ArcSwap`, a reconnect here is observed by every other task on its next call.
//!
//! Cancellation: each retry attempt is races against `token.cancelled()`. If cancellation fires
//! mid-retry, we return an [`Err`] immediately.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

// Retry policy: unbounded retries on transient errors, with exponential backoff capped at 30s.
// Per design feedback ("unbounded RPC reconnections were specifically added to handle longer
// RPC downtime"), we ride out the outage rather than crash the task. Cancellation is observed
// at every `tokio::select!` arm on `token.cancelled()`, so shutdown propagates instantly.
const BACKOFF_START_MS: u64 = 100;
const BACKOFF_CAP_MS: u64 = 30_000;

/// Retry an async closure that takes a `&Arc<cc_client::Client>` and returns
/// `Result<T, cc_client::Error>`. On transient errors, `cc3.reconnect()` is invoked between
/// attempts so the shared `ArcSwap` rolls forward for every other task too.
pub async fn with_retries<T, F, Fut>(
    cc3: &Arc<cc_client::Client>,
    token: &CancellationToken,
    mut f: F,
) -> Result<T, cc_client::Error>
where
    F: FnMut(Arc<cc_client::Client>) -> Fut,
    Fut: std::future::Future<Output = Result<T, cc_client::Error>>,
{
    let mut delay_ms: u64 = BACKOFF_START_MS;
    let mut attempt: usize = 0;

    loop {
        if token.is_cancelled() {
            return Err(cc_client::Error::from(subxt::Error::Other(
                "cancelled".into(),
            )));
        }

        let res = tokio::select! {
            _ = token.cancelled() => return Err(cc_client::Error::from(subxt::Error::Other(
                "cancelled".into(),
            ))),
            r = f(cc3.clone()) => r,
        };

        match res {
            Ok(v) => return Ok(v),
            Err(err) if is_transient(&err) => {
                tracing::warn!(
                    attempt,
                    delay_ms,
                    ?err,
                    "🔁 transient cc3 error — reconnecting and retrying"
                );
                // Best-effort reconnect; the next attempt's classifier sees the fresh
                // connection or re-classifies the same transient error. We *log* the failure at
                // debug so a permanent build_inner failure (e.g. malformed URL) doesn't vanish
                // — `is_transient` may not catch every error variant from `build_inner`, and
                // we'd otherwise spin silently.
                if let Err(err) = cc3.reconnect().await {
                    tracing::debug!(?err, "cc3 reconnect attempt failed inside with_retries");
                }
                tokio::select! {
                    _ = token.cancelled() => return Err(cc_client::Error::from(
                        subxt::Error::Other("cancelled".into()),
                    )),
                    () = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {}
                }
                attempt = attempt.saturating_add(1);
                delay_ms = (delay_ms.saturating_mul(2)).min(BACKOFF_CAP_MS);
            }
            Err(err) => return Err(err),
        }
    }
}

/// Classifies a `cc_client::Error` as transient (worth a reconnect+retry) vs. permanent.
///
/// Transient covers WS disconnects, JSON-RPC connection errors, IO errors. Permanent covers
/// decoding errors, dispatch errors, anything wrapping a runtime/module error.
pub fn is_transient(err: &cc_client::Error) -> bool {
    // cc_client::Error exposes its subxt error through `SubxtError(subxt::Error)` (the variant
    // is plain `SubxtError`, not `Subxt`).
    let subxt_err = match err {
        cc_client::Error::SubxtError(e) => e,
        cc_client::Error::RpcError(rpc_err) => return is_transient_rpc(rpc_err),
        // All other variants are domain-level (decoding, dispatch, missing data, transaction
        // outcomes) — permanent for the purpose of "retry & reconnect".
        _ => return false,
    };

    match subxt_err {
        subxt::Error::Rpc(rpc_err) => is_transient_rpc(rpc_err),
        subxt::Error::Io(_) => true,
        // Codec / decoding / runtime / metadata / etc.: permanent.
        _ => false,
    }
}

fn is_transient_rpc(rpc_err: &subxt::error::RpcError) -> bool {
    use subxt::error::RpcError;
    match rpc_err {
        RpcError::DisconnectedWillReconnect(_) | RpcError::SubscriptionDropped => true,
        // jsonrpsee transport errors live behind a dyn Error here; we sniff the printed form
        // since jsonrpsee doesn't expose a stable type to match against from subxt's surface.
        // Keywords cover the canonical transport-layer signals plus server-side transient
        // messages that gateways/load-balancers typically emit at shutdown or overload —
        // these arrive as `JsonRpseeError::Call(_)` (a *structured* JSON-RPC error rather
        // than a transport drop) and would otherwise be misclassified as permanent.
        RpcError::ClientError(boxed) => {
            let s = boxed.to_string().to_ascii_lowercase();
            s.contains("transport")
                || s.contains("connection")
                || s.contains("disconnected")
                || s.contains("closed")
                || s.contains("restart required") // jsonrpsee `RestartNeeded` Display
                // jsonrpsee 0.24 surfaces a *clean* WS close (the server hung up between our
                // request and its response) as `Error::Custom("Error reason could not be found.
                // This is a bug. Please open an issue.")`. It reads like a permanent bug but is
                // really a transport drop — the same one PR #1034 had to special-case for v1.
                // Without this it classifies permanent → `with_retries` gives up → fail-fast →
                // pod restart, defeating the unbounded ride-out policy above.
                || s.contains("error reason could not be found")
                // jsonrpsee's `TransportReceiver dropped` on the same clean-close path; caught
                // incidentally by "transport" above but matched explicitly so it can't regress.
                || s.contains("transportreceiver dropped")
                || s.contains("timeout")
                || s.contains("deadline_exceeded")
                || s.contains("unavailable")        // 503 / generic::unavailable
                || s.contains("going down")          // gateway graceful-shutdown banner
                || s.contains("shutdown")            // common server lifecycle term
                || s.contains("shutting")            // "shutting down"
                || s.contains("rate limit")          // 429 from fronting LB
                || s.contains("too many requests")
        }
        RpcError::RequestRejected(_) | RpcError::InsecureUrl(_) => false,
        _ => false,
    }
}
