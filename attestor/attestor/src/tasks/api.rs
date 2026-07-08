//! Prometheus metrics + liveness HTTP endpoints.
//!
//! Spawned *before* the long startup phases (election wait, BLS fetch), not with the other
//! tasks: a k8s livenessProbe that gets connection-refused throughout a legitimate days-long
//! election wait forces operators to choose between kill-loops and a huge
//! `initialDelaySeconds` that blunts real wedge detection. `/health` therefore serves
//! `starting` (200) from the moment the cc3 client exists; `/metrics` is degraded (503) until
//! the metrics registry — which depends on post-election chain state — is built and dropped
//! into the [`std::sync::OnceLock`] slot.

use std::sync::Arc;

use crate::error::Error;

#[derive(builder::Builder, Clone, Debug)]
pub struct Config {
    pub port: u16,
}

struct AppState {
    /// Filled by `lib.rs` once the metrics registry exists (it needs post-election chain state).
    metrics: Arc<std::sync::OnceLock<metrics::Metrics>>,
    health: crate::health::SharedHealth,
    cc3: Arc<cc_client::Client>,
}

pub async fn run(
    token: tokio_util::sync::CancellationToken,
    health: crate::health::SharedHealth,
    cc3: Arc<cc_client::Client>,
    metrics: Arc<std::sync::OnceLock<metrics::Metrics>>,
    listener: tokio::net::TcpListener,
) -> Result<(), Error> {
    let state = Arc::new(AppState {
        metrics,
        health,
        cc3,
    });

    let router = axum::Router::new()
        .route("/metrics", axum::routing::get(handle_metrics))
        .route("/health", axum::routing::get(handle_health))
        .with_state(state);

    let address = listener.local_addr().map_err(Error::Io)?;
    tracing::info!(?address, "📌 api server up");

    tokio::select! {
        _ = token.cancelled() => Ok(()),
        res = axum::serve(listener, router) => {
            res?;
            Ok(())
        }
    }
}

/// Liveness probe for k8s. Deliberately cheap — no hardware refresh, no RPC — so it stays
/// responsive even while the node is under load. 200 = alive, 503 = restart me. The body carries
/// the [`crate::health::Liveness`] reason for operator visibility.
async fn handle_health(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    let liveness = state.health.liveness(&state.cc3);
    let status = if liveness.is_alive() {
        axum::http::StatusCode::OK
    } else {
        tracing::error!(%liveness, "🚑 liveness check failed — signalling unhealthy to k8s");
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (status, liveness.to_string())
}

async fn handle_metrics(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> axum::response::Response {
    // Not ready until lib.rs fills the slot (post-election): scrapers get an explicit 503
    // rather than an empty registry that would zero every panel.
    let Some(metrics) = state.metrics.get() else {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::SERVICE_UNAVAILABLE)
            .body(axum::body::Body::from("metrics not ready (starting)"))
            .unwrap();
    };

    metrics.update_hardware().await;

    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            common::constants::METRICS_HEADER,
        )
        .body(axum::body::Body::from(metrics.encode()))
        .unwrap()
}
