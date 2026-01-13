use axum::{http::StatusCode, response::IntoResponse};
use prometheus::TextEncoder;
use std::sync::Arc;

/// Handler for the /metrics endpoint
/// Returns Prometheus-formatted metrics
pub async fn metrics_handler(registry: Arc<prometheus::Registry>) -> impl IntoResponse {
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    match encoder.encode_to_string(&metric_families) {
        Ok(metrics) => (
            StatusCode::OK,
            [(
                "content-type",
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )],
            metrics,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to encode metrics: {e}"),
        )
            .into_response(),
    }
}
