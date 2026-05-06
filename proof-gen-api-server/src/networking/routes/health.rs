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
// 15 minute grace window on restart
const GRACE_WINDOW_DURATION: Duration = Duration::from_secs(15 * 60);

/// Health check response schema for OpenAPI
#[derive(Serialize, ToSchema)]
pub struct HealthCheckResponse {
    status: String,
    cc3_rpc_connected: bool,
    eth_rpc_connected: bool,
    /// `true` when the proof gen server has observed an attestation event
    /// from CC3 within the configured `attestation_liveness_timeout_minutes`.
    attestation_event_listener_alive: bool,
    total_proof_requests: i64,
    uptime_seconds: u64,
}

/// Main health check endpoint - validates upstream services and CC3
/// attestation event liveness.
///
/// Returns HTTP 200 when everything is healthy, HTTP 503 when connectivity is
/// in a disrupted state or attestation liveness is interrupted.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "Service health status", body = HealthCheckResponse),
        (status = 503, description = "Service unhealthy", body = HealthCheckResponse)
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

    let attestation_event_listener_alive = service.check_attestation_event_timer().await.is_ok();

    let healthy = cc3_connected && eth_connected && attestation_event_listener_alive;
    let status = if healthy {
        "healthy".to_string()
    } else {
        "degraded".to_string()
    };

    // If we are in our re-start grace window, then don't use a failure
    // status code. This would trigger another restart through k8s.
    let in_grace_window: bool = Duration::from_secs(service.uptime_seconds()) < GRACE_WINDOW_DURATION;
    let http_status = if healthy || in_grace_window {
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
            total_proof_requests,
            uptime_seconds: service.uptime_seconds(),
        }),
    )
}
