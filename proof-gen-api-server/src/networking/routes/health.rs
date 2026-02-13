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
    total_proof_requests: i64,
    uptime_seconds: u64,
}

/// Main health check endpoint - validates upstream services
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses((status = 200, description = "Service health status", body = HealthCheckResponse))
)]
pub async fn health_check(
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Json<HealthCheckResponse> {
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

    let status = if cc3_connected && eth_connected {
        "healthy".to_string()
    } else {
        "degraded".to_string()
    };

    Json(HealthCheckResponse {
        status,
        cc3_rpc_connected: cc3_connected,
        eth_rpc_connected: eth_connected,
        total_proof_requests,
        uptime_seconds: service.uptime_seconds(),
    })
}
