pub use prometheus_std::{
    Counter, Encoder, Error, Gauge, GaugeVec, IntCounter, IntGauge, Opts, Registry, TextEncoder,
};

use axum::{
    extract::State,
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};

use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error};

pub trait PrometheusRegister<T: Sized = Self>: Sized {
    const DESCRIPTION: &'static str;
    fn register(registry: &Registry) -> Result<Self, Error>;
}

pub fn register_metrics<T: PrometheusRegister>(prometheus_registry: &Arc<Registry>) -> Option<T> {
    match T::register(prometheus_registry) {
        Ok(metrics) => {
            debug!(target: "prometheus", "📈 Registered {} metrics", T::DESCRIPTION);
            Some(metrics)
        }
        Err(err) => {
            error!(
                target: "prometheus",
                "📈 Failed to register {} metrics: {:?}",
                T::DESCRIPTION,
                err
            );
            None
        }
    }
}

#[derive(Debug)]
pub struct HttpServer {
    pub prometheus_registry: Arc<Registry>,
    pub bind_address: String,
    pub port: u16,
}

impl HttpServer {
    pub async fn run_http_server(self: Arc<Self>) {
        let app = self.app();

        let address_str = format!("{}:{}", self.bind_address, self.port);

        let addr: SocketAddr = address_str.parse().expect("Invalid address or port");

        tracing::info!(
            "🚀 Starting prometheus metrics server on http://{}/metrics",
            addr
        );

        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

        axum::serve(listener, app).await.unwrap();
    }

    pub fn app(&self) -> Router {
        Router::new()
            .route("/metrics", get(Self::metrics_handler))
            .with_state(self.prometheus_registry.clone())
    }

    // Axum handler to serve Prometheus metrics
    #[allow(clippy::unused_async)]
    async fn metrics_handler(State(registry): State<Arc<Registry>>) -> impl IntoResponse {
        let metric_families = registry.gather();

        let text_encoder = TextEncoder::new();
        let mut buffer = Vec::new();
        if let Err(err) = text_encoder.encode(&metric_families[..], &mut buffer) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Encoding error: {err:?}"),
            )
                .into_response();
        }

        match String::from_utf8(buffer) {
            Ok(metrics_text) => (
                [(
                    axum::http::header::CONTENT_TYPE,
                    HeaderValue::from_static("text/plain"),
                )],
                metrics_text,
            )
                .into_response(),
            Err(err) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("UTF-8 conversion error: {err:?}"),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{register_metrics, HttpServer, PrometheusRegister};
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use prometheus_std::{Error, GaugeVec, Opts, Registry};
    use std::sync::Arc;
    use tower::util::ServiceExt;

    struct TestMetrics {
        #[allow(dead_code)]
        count: GaugeVec,
    }

    impl PrometheusRegister for TestMetrics {
        const DESCRIPTION: &'static str = "test";

        fn register(registry: &Registry) -> Result<Self, Error> {
            let count = GaugeVec::new(Opts::new("count", "The count to test"), &["count_key"])?;

            registry.register(Box::new(count.clone()))?;

            Ok(Self { count })
        }
    }

    #[test]
    fn should_register_metrics() {
        let registry = Arc::new(Registry::new());
        assert!(register_metrics::<TestMetrics>(&registry.clone()).is_some());
    }

    #[tokio::test]
    async fn prometheus_metrics_are_correctly_set() {
        let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
        let metrics: Option<TestMetrics> = register_metrics(&prometheus_registry.clone());

        if let Some(metrics) = metrics {
            metrics
                .count
                .with_label_values(&["1".to_string()])
                .set(2_f64);
        }

        let http_server = HttpServer {
            prometheus_registry,
            bind_address: "0.0.0.0".to_string(),
            port: 9100,
        };

        let app = http_server.app();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let headers = &response.headers()["content-type"].clone();
        let expected_headers = "text/plain";

        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8(bytes.to_vec()).unwrap();
        let expected_body =
            "# HELP count The count to test\n# TYPE count gauge\ncount{count_key=\"1\"} 2\n"
                .to_string();

        assert_eq!(body_str, expected_body);
        assert_eq!(headers, expected_headers);
    }
}
