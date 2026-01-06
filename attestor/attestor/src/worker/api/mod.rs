//! Public API endpoints for the attestor binary. By default endpoints are exposed on
//! [`DEFAULT_METRICS_PORT`].
//!
//! # Endpoints
//!
//! ## `/metrics`
//!
//! [Prometheus] metrics endpoint, follows the [openmetrics] standard, see [`metrics`] for more
//! information.
//!
//! [`DEFAULT_METRICS_PORT`]: common::constants::DEFAULT_METRICS_PORT
//! [Prometheus]: prometheus_client
//! [openmetrics]: https://openmetrics.io/

pub mod metrics;

use crate::prelude::*;

#[derive(attestor_macro::Builder)]
pub struct Config {
    #[specify_later]
    metrics: common::types::Metrics,
    port: u16,
}

struct AppState {
    metrics: common::types::Metrics,
}

pub(crate) struct WorkerApi {
    metrics: common::types::Metrics,
    port: u16,
}

impl WorkerApi {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            metrics: config.metrics,
            port: config.port,
        }
    }
}

impl super::Worker for WorkerApi {
    fn task(
        self,
        shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> impl std::future::Future<Output = common::types::Result<()>> {
        async move {
            let state = AppState {
                metrics: self.metrics,
            };

            let router = axum::Router::new()
                .route("/metrics", axum::routing::get(handle_metrics))
                .with_state(std::sync::Arc::new(state));

            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
            let address = listener.local_addr().unwrap();

            tracing::info!(?address, "📌 Staring api server");

            tokio::select! {
                res = axum::serve(listener, router) => res?,
                _ = shutdown => {}
            }

            Ok(())
        }
    }
}

async fn handle_metrics(
    axum::extract::State(state): axum::extract::State<std::sync::Arc<AppState>>,
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
