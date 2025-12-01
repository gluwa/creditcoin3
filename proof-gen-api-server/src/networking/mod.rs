use axum::{routing::get, Extension, Router};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot::Receiver;

use crate::services::continuity_service::ContinuityService;
use routes::{continuity, health};

pub mod routes;

pub fn build_app(service: Arc<ContinuityService>) -> Router {
    Router::new()
        .route("/api/v1/health", get(health::health_check))
        .route(
            "/api/v1/proof/{chain_key}/{header_number}",
            get(continuity::get_continuity_proof),
        )
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(continuity::get_continuity_proof_with_tx),
        )
        .route(
            "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
            get(continuity::get_proof_by_tx_hash),
        )
        .layer(Extension(service))
}

pub async fn run_http_server(
    app: Router,
    addr: &str,
    shutdown_rx: Receiver<()>,
) -> anyhow::Result<()> {
    let addr: SocketAddr = addr.parse().expect("Invalid BIND_ADDR format");
    // Bind address (default in config.rs = 0.0.0.0:3100)
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
