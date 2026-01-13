use axum::{http::Method, routing::get, Extension, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot::Receiver;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::Level;

use crate::services::continuity_service::ContinuityService;
use routes::{continuity, health, metrics};

pub mod middleware;
pub mod routes;

pub fn build_app(
    service: Arc<ContinuityService>,
    chain_key: u64,
    prometheus_registry: Arc<prometheus::Registry>,
) -> Router {
    // Configure CORS to allow browser-based applications to access the API
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    Router::new()
        .route("/api/v1/health", get(health::health_check))
        .route("/health/live", get(health::liveness_check))
        .route("/health/ready", get(health::readiness_check))
        .route(
            "/api/v1/proof/{chain_key}/{header_number}",
            get(continuity::get_proof),
        )
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(continuity::get_proof_with_tx),
        )
        .route(
            "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
            get(continuity::get_proofs_by_tx_hash),
        )
        .route(
            "/metrics",
            get(move || metrics::metrics_handler(prometheus_registry.clone())),
        )
        .layer(Extension(service))
        .layer(axum::middleware::from_fn_with_state(
            chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = chain_key;
                async move {
                    crate::networking::middleware::chain_key_validator_middleware(
                        request, next, chain_key,
                    )
                    .await
                }
            },
        ))
        // CORS must be outside the middleware so error responses also get CORS headers
        .layer(cors)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    tracing::span!(
                        Level::INFO,
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                    )
                })
                .on_request(|_request: &axum::http::Request<_>, _span: &tracing::Span| {
                    tracing::event!(Level::INFO, "Incoming request");
                })
                .on_response(
                    |_response: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::event!(
                            Level::INFO,
                            latency_ms = latency.as_millis(),
                            status = %_response.status(),
                            "Request completed"
                        );
                    },
                ),
        )
}

pub async fn run_http_server(
    app: Router,
    addr: SocketAddr,
    shutdown_rx: Receiver<()>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let shutdown_closure = async move {
        // this future completes when we send on http_shutdown_tx
        let _ = shutdown_rx.await;
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_closure)
        .await?;
    Ok(())
}
