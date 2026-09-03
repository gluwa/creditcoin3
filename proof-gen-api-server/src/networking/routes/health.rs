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
    /// `healthy` when this replica can serve proofs: its cc3 caches are advancing and the
    /// source-chain RPC is reachable. `degraded` otherwise.
    status: String,
    /// Live cc3 RPC storage probe across every configured chain. Diagnostic only — a false
    /// here with `cc3_cache_fresh: true` means the subscription is fine but a point-in-time
    /// read failed; the reason is logged at WARN as `cc3 RPC probe failed`.
    cc3_rpc_connected: bool,
    eth_rpc_connected: bool,
    /// Whether the cc3 event subscription advanced every chain's cache within the freshness
    /// window. This is what proof serving actually depends on.
    cc3_cache_fresh: bool,
    /// Seconds since the least-recently-advanced chain's cache last changed.
    cc3_cache_age_seconds: u64,
    uptime_seconds: u64,
}

/// Run one upstream probe under the shared timeout, logging why it failed if it does.
/// The previous implementation reduced the error to a bare `false` via `is_ok_and`, which
/// left no record anywhere of the cause.
async fn probe<F>(name: &'static str, fut: F) -> bool
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    match timeout(HEALTH_CHECK_TIMEOUT, fut).await {
        Ok(Ok(())) => true,
        Ok(Err(err)) => {
            tracing::warn!(probe = name, ?err, "🩺 health probe failed");
            false
        }
        Err(_) => {
            tracing::warn!(
                probe = name,
                timeout_secs = HEALTH_CHECK_TIMEOUT.as_secs(),
                "🩺 health probe timed out"
            );
            false
        }
    }
}

/// Main health check endpoint.
///
/// `status` is driven by what serving proofs actually requires — fresh cc3 caches and a
/// reachable source chain — rather than by the live cc3 storage probe. That probe is kept in
/// the payload and logged on failure, but on its own it was reporting replicas as degraded
/// while they served proofs with zero errors, because the request path reads the caches and
/// never issues that query.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses((status = 200, description = "Service health status", body = HealthCheckResponse))
)]
pub async fn health_check(
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Json<HealthCheckResponse> {
    let (cc3_connected, eth_connected) = tokio::join!(
        probe("cc3_rpc", service.check_cc3_connectivity()),
        probe("eth_rpc", service.check_eth_connectivity()),
    );

    let freshness = service.cc3_cache_freshness();
    if !freshness.fresh {
        tracing::warn!(
            stale_chains = ?freshness.stale_chains,
            max_age_seconds = freshness.max_age_seconds,
            "🩺 cc3 cache stale — event subscription not advancing"
        );
    }

    let status = if freshness.fresh && eth_connected {
        "healthy".to_string()
    } else {
        "degraded".to_string()
    };

    Json(HealthCheckResponse {
        status,
        cc3_rpc_connected: cc3_connected,
        eth_rpc_connected: eth_connected,
        cc3_cache_fresh: freshness.fresh,
        cc3_cache_age_seconds: freshness.max_age_seconds,
        uptime_seconds: service.uptime_seconds(),
    })
}
