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

const MAX_ATTEMPTS: usize = 6; // ≈ 100 + 200 + 400 + 800 + 1600 + 3200 ms = ~6.3s capped

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
    let mut delay_ms: u64 = 100;
    let mut last_err: Option<cc_client::Error> = None;

    for attempt in 0..MAX_ATTEMPTS {
        if token.is_cancelled() {
            return last_err
                .map(Err)
                .unwrap_or_else(|| Err(cc_client::Error::from(subxt::Error::Other(
                    "cancelled".into(),
                ))));
        }

        let res = tokio::select! {
            _ = token.cancelled() => return last_err
                .map(Err)
                .unwrap_or_else(|| Err(cc_client::Error::from(subxt::Error::Other(
                    "cancelled".into(),
                )))),
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
                let _ = cc3.reconnect().await; // best-effort; next attempt will see fresh conn
                last_err = Some(err);
                tokio::select! {
                    _ = token.cancelled() => return last_err.map(Err).unwrap(),
                    _ = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => {}
                }
                delay_ms = (delay_ms.saturating_mul(2)).min(3_200);
            }
            Err(err) => return Err(err),
        }
    }

    Err(last_err.unwrap_or_else(|| {
        cc_client::Error::from(subxt::Error::Other("retries exhausted".into()))
    }))
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
        RpcError::ClientError(boxed) => {
            let s = boxed.to_string().to_ascii_lowercase();
            s.contains("transport")
                || s.contains("connection")
                || s.contains("disconnected")
                || s.contains("closed")
                || s.contains("timeout")
        }
        RpcError::RequestRejected(_) | RpcError::InsecureUrl(_) => false,
        _ => false,
    }
}
