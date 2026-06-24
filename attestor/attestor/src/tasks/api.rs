//! Prometheus metrics + liveness HTTP endpoints.

use std::sync::Arc;

use crate::error::Error;
use crate::shared::Shared;

#[derive(builder::Builder)]
pub struct Config {
    #[specify_later]
    metrics: metrics::Metrics,
    port: u16,
}

struct AppState {
    metrics: metrics::Metrics,
    shared: Arc<Shared>,
}

pub async fn run(shared: Arc<Shared>, cfg: Config) -> Result<(), Error> {
    let state = Arc::new(AppState {
        metrics: cfg.metrics,
        shared: shared.clone(),
    });

    let router = axum::Router::new()
        .route("/metrics", axum::routing::get(handle_metrics))
        .route("/health", axum::routing::get(handle_health))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cfg.port)).await?;
    let address = listener.local_addr().map_err(crate::error::Error::Io)?;
    tracing::info!(?address, "📌 api server up");

    tokio::select! {
        _ = shared.token.cancelled() => Ok(()),
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
    let liveness = state.shared.health.liveness(&state.shared.cc3);
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
) -> impl axum::response::IntoResponse {
    state.metrics.update_hardware().await;

    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            common::constants::METRICS_HEADER,
        )
        .body(axum::body::Body::from(state.metrics.encode()))
        .unwrap()
}
