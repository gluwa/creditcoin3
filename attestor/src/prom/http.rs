use axum::{
    extract::State,
    http::{HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};

use prometheus::{Encoder, Registry, TextEncoder};
use std::net::SocketAddr;
use std::sync::Arc;
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

        tracing::info!("🚀 Starting prometheus metrics server on http://{}", addr);

        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

        axum::serve(listener, app).await.unwrap();
    }

    fn app(&self) -> Router {
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
    use super::HttpServer;
    use crate::prom::{register_metrics, AttestorMetrics};
    use crate::{metric_set, metric_set_labels};
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use prometheus::Registry;
    use std::sync::Arc;

    use tower::util::ServiceExt;

    #[tokio::test]
    async fn prometheus_metrics_are_correctly_set() {
        let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
        let metrics: Option<AttestorMetrics> = register_metrics(&prometheus_registry.clone());

        metric_set!(metrics, attestor_chain_key, 2);

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
        let expected_body = "# HELP attestor_chain_key Attestor chain key\n# TYPE attestor_chain_key gauge\nattestor_chain_key 2\n".to_string();

        assert_eq!(body_str, expected_body);
        assert_eq!(headers, expected_headers);
    }

    #[tokio::test]
    async fn prometheus_metrics_contain_correct_labels_when_set() {
        let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
        let metrics: Option<AttestorMetrics> = register_metrics(&prometheus_registry.clone());

        metric_set_labels!(metrics, source_chain_rpc_url, "localhost:8545", 2, 1);
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

        let bytes = to_bytes(response.into_body(), 1024).await.unwrap();
        let body_str = String::from_utf8(bytes.to_vec()).unwrap();
        let expected_body = "# HELP attestor_chain_key Attestor chain key\n# TYPE attestor_chain_key gauge\nattestor_chain_key 0\n# HELP source_chain_rpc_url Source chain node rpc url\n# TYPE source_chain_rpc_url gauge\nsource_chain_rpc_url{chain_key=\"2\",source_chain_rpc_url=\"localhost:8545\"} 1\n".to_string();

        assert_eq!(body_str, expected_body);
        assert!(body_str.contains("source_chain_rpc_url=\"localhost:8545\"}"));
    }
}
