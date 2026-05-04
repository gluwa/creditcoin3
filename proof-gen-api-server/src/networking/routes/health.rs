use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use utoipa::ToSchema;

use crate::services::continuity_service::ContinuityService;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Health check response schema for OpenAPI
#[derive(Serialize, ToSchema)]
pub struct HealthCheckResponse {
    status: String,
    cc3_rpc_connected: bool,
    eth_rpc_connected: bool,
    /// `true` when the proof gen server has observed an attestation event
    /// from CC3 within the configured `attestation_liveness_timeout_minutes`.
    /// When `false`, the CC3 attestation event listener is considered stalled
    /// and the response is returned with HTTP 503 so a k8s liveness probe can
    /// trigger a restart (CSUB-2039).
    attestation_event_listener_alive: bool,
    /// Seconds since the most recent attestation event was observed from CC3
    /// (or service startup if none have been observed yet).
    seconds_since_last_attestation_event: u64,
    /// Configured maximum tolerated gap between attestation events, in
    /// seconds. Returned for operator visibility.
    attestation_liveness_timeout_seconds: u64,
    total_proof_requests: i64,
    uptime_seconds: u64,
}

/// Main health check endpoint - validates upstream services and CC3
/// attestation event liveness.
///
/// Returns HTTP 200 when everything is healthy, HTTP 503 when the CC3
/// attestation event listener appears stalled (no `BlockAttested` event
/// received in the last `attestation_liveness_timeout_minutes`). Operators
/// should wire this endpoint into a liveness probe so the orchestrator
/// restarts the server and re-establishes the subscription.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "Service health status", body = HealthCheckResponse),
        (
            status = 503,
            description = "Service unhealthy (e.g. CC3 attestation event listener stalled)",
            body = HealthCheckResponse
        )
    )
)]
pub async fn health_check(
    Extension(service): Extension<Arc<ContinuityService>>,
) -> impl IntoResponse {
    // Check RPC connectivity in parallel with timeout
    let (total_requests, cc3_connected, eth_connected) = tokio::join!(
        async { timeout(HEALTH_CHECK_TIMEOUT, service.get_proofs_counts()).await },
        async {
            timeout(HEALTH_CHECK_TIMEOUT, service.check_cc3_connectivity())
                .await
                .is_ok_and(|result| result.is_ok())
        },
        async {
            timeout(HEALTH_CHECK_TIMEOUT, service.check_eth_connectivity())
                .await
                .is_ok_and(|result| result.is_ok())
        }
    );

    let total_proof_requests = match total_requests {
        Ok(Ok(count)) => count,
        Ok(Err(_)) | Err(_) => 0,
    };

    // Attestation liveness check (CSUB-2039). Cheap in-memory check, no need
    // to gate it behind HEALTH_CHECK_TIMEOUT.
    let liveness_check = service.check_attestation_event_timer().await;
    let attestation_event_listener_alive = liveness_check.is_ok();
    let seconds_since_last_attestation_event =
        service.time_since_last_attestation_event().await.as_secs();
    let attestation_liveness_timeout_seconds = service.attestation_liveness_timeout().as_secs();

    if let Err(ref err) = liveness_check {
        tracing::error!(
            elapsed_secs = seconds_since_last_attestation_event,
            timeout_secs = attestation_liveness_timeout_seconds,
            error = %err,
            "CC3 attestation event listener appears stalled; reporting unhealthy from /api/v1/health so the orchestrator can restart this proof gen server (CSUB-2039)"
        );
    }

    let healthy = cc3_connected && eth_connected && attestation_event_listener_alive;
    let status = if healthy {
        "healthy".to_string()
    } else {
        "degraded".to_string()
    };

    // Surface attestation-listener stalls as 503 specifically so liveness
    // probes can act on it. CC3/ETH RPC blips alone keep the legacy 200
    // behaviour to avoid disrupting existing readiness probes.
    let http_status = if attestation_event_listener_alive {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        http_status,
        Json(HealthCheckResponse {
            status,
            cc3_rpc_connected: cc3_connected,
            eth_rpc_connected: eth_connected,
            attestation_event_listener_alive,
            seconds_since_last_attestation_event,
            attestation_liveness_timeout_seconds,
            total_proof_requests,
            uptime_seconds: service.uptime_seconds(),
        }),
    )
}
