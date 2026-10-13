//! Request admission: a process-wide and a per-chain cap on proof requests in flight, plus an
//! end-to-end deadline per request.
//!
//! `max_batch_size` bounds the work inside one batch, not how many requests run at once. With
//! R simultaneous cold requests the old router let roughly `R × (20 + max_batch_size)`
//! block-level operations run, each with its own RPC retries, and no request ever timed out.
//! Refusing the excess up front with a `503` is cheaper for everyone: the caller retries with
//! back-off, and the requests that were admitted actually finish.
//!
//! Health, readiness and metrics are exempt so probes and recovery keep working under load.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use serde_json::json;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::config::AdmissionConfig;
use crate::networking::middleware::extract_chain_key_from_path;
use crate::prom::{labels::Rejection, ProofGenMetrics};

pub struct Admission {
    global: Arc<Semaphore>,
    per_chain: HashMap<u64, Arc<Semaphore>>,
    request_timeout: Duration,
    metrics: Arc<ProofGenMetrics>,
}

impl Admission {
    pub fn new(
        config: &AdmissionConfig,
        chain_keys: impl IntoIterator<Item = u64>,
        metrics: Arc<ProofGenMetrics>,
    ) -> Self {
        Self {
            global: Arc::new(Semaphore::new(config.max_in_flight_requests.get())),
            per_chain: chain_keys
                .into_iter()
                .map(|k| {
                    (
                        k,
                        Arc::new(Semaphore::new(config.max_in_flight_per_chain.get())),
                    )
                })
                .collect(),
            request_timeout: config.request_timeout,
            metrics,
        }
    }

    /// Try to admit one proof request for `chain_key`. `Err` names the limit that was hit.
    fn try_admit(&self, chain_key: u64) -> Result<Permits, Rejection> {
        let global = self
            .global
            .clone()
            .try_acquire_owned()
            .map_err(|_| Rejection::Overloaded)?;
        let chain = match self.per_chain.get(&chain_key) {
            Some(sem) => Some(
                sem.clone()
                    .try_acquire_owned()
                    .map_err(|_| Rejection::ChainOverloaded)?,
            ),
            // Unknown chain: the chain-key validator answers 400 further in; only the global
            // permit applies.
            None => None,
        };
        Ok(Permits {
            _global: global,
            _chain: chain,
        })
    }
}

/// Counts one admitted request on `requests_in_flight` for exactly as long as it lives.
struct InFlight<'a> {
    metrics: &'a ProofGenMetrics,
}

impl<'a> InFlight<'a> {
    fn enter(metrics: &'a ProofGenMetrics) -> Self {
        metrics.request_admitted();
        Self { metrics }
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.metrics.request_finished();
    }
}

struct Permits {
    _global: OwnedSemaphorePermit,
    _chain: Option<OwnedSemaphorePermit>,
}

/// Only proof endpoints are admission-controlled; everything else passes untouched.
fn is_proof_request(request: &Request) -> bool {
    request.uri().path().starts_with("/api/v1/proof")
}

pub async fn admission_middleware(
    Extension(admission): Extension<Arc<Admission>>,
    request: Request,
    next: Next,
) -> Response {
    if !is_proof_request(&request) {
        return next.run(request).await;
    }
    let chain_key = extract_chain_key_from_path(request.uri()).unwrap_or(u64::MAX);
    let _permits = match admission.try_admit(chain_key) {
        Ok(permits) => permits,
        Err(reason) => {
            admission.metrics.request_rejected(reason);
            tracing::warn!(
                chain_key,
                ?reason,
                "🚦 proof request refused by admission control"
            );
            return rejected(
                StatusCode::SERVICE_UNAVAILABLE,
                match reason {
                    Rejection::ChainOverloaded => "ChainOverloaded",
                    _ => "Overloaded",
                },
                "too many proof requests in flight; retry with back-off",
            );
        }
    };

    // Guarded like the permits: a client that disconnects makes axum drop this future
    // mid-request, and a handler panic unwinds through it; either must still take the request
    // back off the gauge, or `requests_in_flight` only ever climbs.
    let _in_flight = InFlight::enter(&admission.metrics);
    let outcome = tokio::time::timeout(admission.request_timeout, next.run(request)).await;
    match outcome {
        Ok(response) => response,
        Err(_elapsed) => {
            admission.metrics.request_rejected(Rejection::Timeout);
            tracing::warn!(
                chain_key,
                timeout = ?admission.request_timeout,
                "⏱️ proof request exceeded its deadline"
            );
            rejected(
                StatusCode::GATEWAY_TIMEOUT,
                "RequestTimeout",
                "proof request exceeded the server's deadline",
            )
        }
    }
}

fn rejected(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        [(header::RETRY_AFTER, "1")],
        axum::Json(json!({
            "code": code,
            "message": message,
            "retriable": true,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_flight(metrics: &ProofGenMetrics) -> Option<String> {
        metrics
            .encode()
            .lines()
            .find(|l| l.starts_with("proof_gen_requests_in_flight"))
            .map(str::to_owned)
    }

    #[tokio::test]
    async fn a_cancelled_request_comes_back_off_the_in_flight_gauge() {
        let metrics = ProofGenMetrics::new(&[1]);
        let baseline = in_flight(&metrics);
        let request = async {
            let _in_flight = InFlight::enter(&metrics);
            std::future::pending::<()>().await;
        };
        // Poll once so the guard is live, then drop the future as axum does on disconnect.
        let mut request = Box::pin(request);
        assert!(futures::poll!(request.as_mut()).is_pending());
        assert_ne!(in_flight(&metrics), baseline, "counted while in flight");
        drop(request);
        assert_eq!(in_flight(&metrics), baseline, "released on cancellation");
    }
}
