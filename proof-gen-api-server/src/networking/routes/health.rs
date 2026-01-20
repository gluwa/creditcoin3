use axum::{http::StatusCode, Extension, Json};
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use crate::services::continuity_service::ContinuityService;

const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Main health check endpoint - validates upstream services
pub async fn health_check(Extension(service): Extension<Arc<ContinuityService>>) -> Json<Value> {
    // Check RPC connectivity in parallel with timeout
    let (cache_stats, cc3_connected, eth_connected) = tokio::join!(
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

    let (cache_hits, cache_misses, total) = match cache_stats {
        Ok(Ok((hits, misses, total))) => (hits, misses, total),
        Ok(Err(_)) | Err(_) => (0, 0, 0),
    };

    let overall_healthy = cc3_connected && eth_connected;
    let status = if overall_healthy {
        "healthy"
    } else {
        "degraded"
    };

    let body = json!({
        "status": status,
        "cc3_rpc_connected": cc3_connected,
        "eth_rpc_connected": eth_connected,
        "cache_hits": cache_hits,
        "cache_misses": cache_misses,
        "total_requests": total,
        "uptime_seconds": service.uptime_seconds()
    });

    Json(body)
}

/// Liveness probe - returns 200 if service process is running
pub async fn liveness_check() -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "status": "alive",
            "timestamp": Utc::now().timestamp()
        })),
    )
}

/// Readiness probe - returns 200 if service can handle requests  
pub async fn readiness_check(
    Extension(service): Extension<Arc<ContinuityService>>,
) -> (StatusCode, Json<Value>) {
    let start_time = Utc::now();

    // Check RPC connectivity in parallel
    let (cc3_ready, eth_ready) = tokio::join!(
        async {
            timeout(Duration::from_secs(2), service.check_cc3_connectivity())
                .await
                .is_ok_and(|result| result.is_ok())
        },
        async {
            timeout(Duration::from_secs(2), service.check_eth_connectivity())
                .await
                .is_ok_and(|result| result.is_ok())
        }
    );

    let ready = cc3_ready && eth_ready;
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(json!({
            "status": if ready { "ready" } else { "not_ready" },
            "timestamp": start_time.timestamp(),
            "cc3_rpc_ready": cc3_ready,
            "eth_rpc_ready": eth_ready
        })),
    )
}
