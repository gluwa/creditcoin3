use axum::{
    http::Method,
    response::Redirect,
    routing::{get, post},
    Extension, Router,
};
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot::Receiver;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::Level;

use crate::prom::{handle_metrics_response, Metrics, ProofGenMetrics};
use crate::services::continuity_service::ContinuityService;
use routes::{attested_height, continuity, health};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

pub mod admission;
pub mod extract;
pub mod middleware;
pub mod openapi;
pub mod routes;

/// Build the router with default [`crate::config::AdmissionConfig`].
pub fn build_app(
    service: Arc<ContinuityService>,
    allowed_chain_keys: HashSet<u64>,
    prom_metrics: Arc<ProofGenMetrics>,
) -> Router {
    build_app_with_admission(
        service,
        allowed_chain_keys,
        prom_metrics,
        crate::config::AdmissionConfig::default(),
    )
}

pub fn build_app_with_admission(
    service: Arc<ContinuityService>,
    allowed_chain_keys: HashSet<u64>,
    prom_metrics: Arc<ProofGenMetrics>,
    admission_config: crate::config::AdmissionConfig,
) -> Router {
    let metrics: Metrics = prom_metrics.clone() as Metrics;
    let admission = Arc::new(admission::Admission::new(
        &admission_config,
        allowed_chain_keys.iter().copied(),
        prom_metrics.clone(),
    ));
    let allowed_chain_keys = Arc::new(allowed_chain_keys);
    // Configure CORS to allow browser-based applications to access the API
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let router = Router::new()
        .route("/", get(|| async { Redirect::permanent("/api/swagger") }))
        .route("/api/v1/health", get(health::health_check))
        .route("/livez", get(health::livez))
        .route("/readyz", get(health::readyz))
        .route(
            "/api/v1/attested-height/{chain_key}",
            get(attested_height::attested_height),
        )
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(continuity::get_proof_with_tx),
        )
        .route(
            "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
            get(continuity::get_proof_by_tx_hash),
        )
        .route(
            "/api/v1/proof-batch/{chain_key}",
            post(continuity::get_proof_batch),
        )
        .route(
            "/api/v1/proof-batch-by-tx/{chain_key}",
            post(continuity::get_proof_batch_by_tx_hash),
        )
        .route(
            "/metrics",
            get(
                |Extension(metrics): Extension<Arc<ProofGenMetrics>>| async move {
                    handle_metrics_response(metrics)
                },
            ),
        )
        .merge(
            SwaggerUi::new("/api/swagger")
                .url("/api/swagger/openapi.json", openapi::ApiDoc::openapi())
                .config(openapi::swagger_config()),
        )
        .layer(Extension(service))
        .layer(Extension(prom_metrics.clone()))
        // Admission sits directly around the handlers: rejected requests still pass through
        // the request-metrics, chain-key and CORS layers below (outer), so they are counted
        // and carry CORS headers like any other response.
        .layer(axum::middleware::from_fn(admission::admission_middleware))
        .layer(Extension(admission));

    router
        // Request metrics middleware - tracks count, duration, and sizes
        // Note: Extension(metrics) must be AFTER (outer) the middleware so it's available
        .layer(axum::middleware::from_fn(
            middleware::request_metrics_middleware,
        ))
        .layer(Extension(metrics.clone()))
        .layer({
            let allowed_chain_keys = allowed_chain_keys.clone();
            axum::middleware::from_fn(move |request, next| {
                let allowed_chain_keys = allowed_chain_keys.clone();
                async move {
                    crate::networking::middleware::chain_key_validator_middleware(
                        request,
                        next,
                        allowed_chain_keys,
                    )
                    .await
                }
            })
        })
        // CORS must be outside the middleware so error responses also get CORS headers
        .layer(cors)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<_>| {
                    let request_id = uuid::Uuid::new_v4();

                    tracing::span!(
                        Level::INFO,
                        "http_request",
                        method = %request.method(),
                        uri = %request.uri(),
                        request_id = %request_id,
                    )
                })
                .on_request(|_request: &axum::http::Request<_>, _span: &tracing::Span| {
                    tracing::event!(Level::INFO, "🌐 ⬇️  Incoming request");
                })
                .on_response(
                    |_response: &axum::http::Response<_>,
                     latency: std::time::Duration,
                     _span: &tracing::Span| {
                        tracing::event!(
                            Level::INFO,
                            latency_ms = latency.as_millis(),
                            status = %_response.status(),
                            "🌐 ✅ Request completed"
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
