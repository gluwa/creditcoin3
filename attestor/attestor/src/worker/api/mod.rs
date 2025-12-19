mod error;
pub(crate) mod metrics;

use crate::prelude::*;
pub use error::*;

#[derive(attestor_macro::Builder)]
pub struct Config {
    #[specify_later]
    metrics: common::types::Metrics,
    port: u16,
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
    axum::extract::State(state): axum::extract::State<common::types::Metrics>,
) -> impl axum::response::IntoResponse {
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
