use axum::{routing::get, Router};
use routes::{continuity, health};
use std::net::SocketAddr;
use thiserror::Error;
use tokio::net::TcpListener;

pub mod routes;

#[derive(Error, Debug)]
pub enum ApiError {
    #[error("Invalid request: {0}")]
    BadRequest(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Internal server error")]
    InternalError,
}

pub fn build_app() -> Router {
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
}

pub async fn run_http_server(app: Router, addr: &str) -> anyhow::Result<()> {
    let addr: SocketAddr = addr.parse().expect("Invalid BIND_ADDR format");
    // Bind address (default in config.rs = 0.0.0.0:3100)
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

pub async fn shutdown_signal() {
    use tokio::signal;

    // Ctrl+C
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!("Shutdown signal received");
}
