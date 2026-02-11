//! Public API endpoints for the attestor binary. By default endpoints are exposed on
//! [`DEFAULT_API_PORT`].
//!
//! # Endpoints
//!
//! ## `/metrics`
//!
//! [Prometheus] metrics endpoint, follows the [openmetrics] standard, see [`metrics`] for more
//! information.
//!
//! [`DEFAULT_API_PORT`]: common::constants::DEFAULT_API_PORT
//! [Prometheus]: prometheus_client
//! [openmetrics]: https://openmetrics.io/

mod error;
pub mod metrics;

pub use error::Error;

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
    type Error = Error;

    async fn task(
        self,
        shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> crate::worker::Exit<Error> {
        let state = AppState {
            metrics: self.metrics,
        };

        let router = axum::Router::new()
            .route("/metrics", axum::routing::get(handle_metrics))
            .with_state(std::sync::Arc::new(state));

        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port))
            .await
            .map_interrupt(Error::Io)?;
        let address = listener.local_addr().unwrap();

        tracing::info!(?address, "📌 Starting api server");

        tokio::select! {
            _ = shutdown => {
                Err(Interrupt::Stop)
            }
            res = axum::serve(listener, router) => {
                res.map_interrupt(Error::Io)?;
                Ok(())
            },
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
