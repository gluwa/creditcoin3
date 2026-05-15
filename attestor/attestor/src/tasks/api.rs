//! Prometheus metrics HTTP endpoint.

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
}

pub async fn run(shared: Arc<Shared>, cfg: Config) -> Result<(), Error> {
    let state = Arc::new(AppState { metrics: cfg.metrics });

    let router = axum::Router::new()
        .route("/metrics", axum::routing::get(handle_metrics))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", cfg.port)).await?;
    let address = listener.local_addr().unwrap();
    tracing::info!(?address, "📌 api server up");

    tokio::select! {
        _ = shared.token.cancelled() => Ok(()),
        res = axum::serve(listener, router) => {
            res?;
            Ok(())
        }
    }
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
