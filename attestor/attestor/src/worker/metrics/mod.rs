mod error;
pub(crate) mod store;

use crate::prelude::*;
pub use error::*;

#[derive(attestor_macro::Builder)]
pub struct Config {
    #[specify_later]
    metrics: std::sync::Arc<tokio::sync::Mutex<store::MetricsStore>>,
    port: u16,
}

pub(crate) struct WorkerMetrics {
    metrics: std::sync::Arc<tokio::sync::Mutex<store::MetricsStore>>,
    port: u16,
}

impl WorkerMetrics {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            metrics: config.metrics,
            port: config.port,
        }
    }
}

impl super::Worker for WorkerMetrics {
    fn task(
        self,
        shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> impl std::future::Future<Output = common::types::Result<()>> {
        async move {
            let router = axum::Router::new()
                .route("/metrics", axum::routing::get(handle_metrics))
                .with_state(self.metrics);
            let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", self.port)).await?;
            let address = listener.local_addr().unwrap();

            tracing::info!(?address, "📌 Staring metrics server");

            tokio::select! {
                res = axum::serve(listener, router) => res?,
                _ = shutdown => {}
            }

            Ok(())
        }
    }
}

async fn handle_metrics(
    axum::extract::State(state): axum::extract::State<
        std::sync::Arc<tokio::sync::Mutex<store::MetricsStore>>,
    >,
) -> impl axum::response::IntoResponse {
    let state = state.lock().await;
    let mut buffer = String::new();
    prometheus_client::encoding::text::encode(&mut buffer, &state.registry).unwrap();

    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            axum::http::header::CONTENT_TYPE,
            "application/openmetrics-text; version=1.0.0; charset=utf-8",
        )
        .body(axum::body::Body::from(buffer))
        .unwrap()
}
