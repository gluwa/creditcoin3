use prometheus::{
    register_metrics, Error, Gauge, GaugeVec, HttpServer, Opts, PrometheusRegister, Registry,
};
use std::sync::Arc;

use crate::util::sanitize_url::sanitize_rpc_url_api_key;
use crate::Config;

use crate::{metric_set, metric_set_labels};

/// Starts the Prometheus metrics server and registers the attestor metrics.
/// returns an optional `AttestorMetrics` instance if registration is successful.
pub fn start_prom_server(config: &Config, chain_name: &str) -> Option<AttestorMetrics> {
    let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
    let metrics: Option<AttestorMetrics> = register_metrics(&prometheus_registry.clone());

    // set initial metrics
    metric_set!(metrics, attestor_chain_key, &config.chain_key);
    metric_set_labels!(
        metrics,
        source_chain_rpc_url,
        [
            chain_name,
            &config.chain_key,
            sanitize_rpc_url_api_key(&config.eth_rpc_url)
        ],
        1
    );
    metric_set_labels!(
        metrics,
        cc3next_rpc_url,
        [chain_name, &config.chain_key, &config.cc3_rpc_url],
        1
    );
    // Create http server for metrics
    let http_server = Arc::new(HttpServer {
        prometheus_registry,
        bind_address: config.prometheus_host.clone(),
        port: config.prometheus_port,
    });
    tokio::spawn(http_server.clone().run_http_server());

    metrics
}
#[derive(Clone, Debug)]
pub struct AttestorMetrics {
    pub last_voted_for: GaugeVec,
    pub last_finalized_attestation: GaugeVec,
    pub source_chain_height: GaugeVec,
    pub cc_current_epoch: GaugeVec,
    pub attestor_chain_key: Gauge,
    pub source_chain_rpc_url: GaugeVec,
    pub cc3next_rpc_url: GaugeVec,
}

impl PrometheusRegister for AttestorMetrics {
    const DESCRIPTION: &'static str = "attestor";
    fn register(registry: &Registry) -> Result<Self, Error> {
        let last_voted_for = GaugeVec::new(
            Opts::new(
                "last_block_voted_for",
                "The last block the attestor voted for",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(last_voted_for.clone()))?;

        let last_finalized_attestation = GaugeVec::new(
            Opts::new(
                "last_finalized_attestation",
                "The last finalized attestation header",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(last_finalized_attestation.clone()))?;

        let source_chain_height = GaugeVec::new(
            Opts::new(
                "source_chain_height",
                "The last finalized source chain header",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(source_chain_height.clone()))?;

        let cc_current_epoch = GaugeVec::new(
            Opts::new("cc_current_epoch", "The current epoch of the cc chain"),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(cc_current_epoch.clone()))?;

        let attestor_chain_key = Gauge::new("attestor_chain_key", "Attestor chain key")?;
        registry.register(Box::new(attestor_chain_key.clone()))?;

        let source_chain_rpc_url = GaugeVec::new(
            Opts::new("source_chain_rpc_url", "Source chain node rpc url"),
            &["chain", "chain_key", "source_chain_rpc_url"],
        )?;
        registry.register(Box::new(source_chain_rpc_url.clone()))?;

        let cc3next_rpc_url = GaugeVec::new(
            Opts::new("cc3next_rpc_url", "cc3next node rpc url"),
            &["chain", "chain_key", "cc3next_rpc_url"],
        )?;
        registry.register(Box::new(cc3next_rpc_url.clone()))?;

        Ok(Self {
            last_voted_for,
            last_finalized_attestation,
            source_chain_height,
            cc_current_epoch,
            attestor_chain_key,
            source_chain_rpc_url,
            cc3next_rpc_url,
        })
    }
}

// Note: we use the `format` macro to convert an expr into a `u64`. This will fail,
// if expr does not derive `Display`.
#[macro_export]
macro_rules! metric_set {
    ($metrics:expr, $m:ident, $v:expr) => {{
        let val: u64 = format!("{}", $v).parse().unwrap();

        if let Some(metrics) = $metrics.as_ref() {
            #[allow(clippy::cast_precision_loss)]
            metrics.$m.set(val as f64);
        }
    }};
}

#[macro_export]
macro_rules! metric_set_label {
    ($metrics:expr, $m:ident, $metric_label:expr, $v:expr) => {{
        let val: u64 = format!("{}", $v).parse().unwrap();

        if let Some(metrics) = $metrics.as_ref() {
            #[allow(clippy::cast_precision_loss)]
            metrics
                .$m
                .with_label_values(&[&$metric_label.to_string()])
                .set(val as f64);
        }
    }};
}

#[macro_export]
macro_rules! metric_set_labels {
    ($metrics:expr, $m:ident, [ $( $label:expr ),* $(,)? ], $v:expr) => {{
        let val: u64 = format!("{}", $v).parse().unwrap();

        if let Some(metrics) = $metrics.as_ref() {
            #[allow(clippy::cast_precision_loss)]
            metrics
                .$m
                .with_label_values(&[
                    $( &$label.to_string(), )*
                ])
                .set(val as f64);
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::AttestorMetrics;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use prometheus::{register_metrics, HttpServer, Registry};
    use std::sync::Arc;

    use tower::util::ServiceExt;

    #[tokio::test]
    async fn prometheus_metrics_are_correctly_set_for_attestor() {
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
    async fn prometheus_metrics_contain_correct_labels_when_set_for_attestor() {
        let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
        let metrics: Option<AttestorMetrics> = register_metrics(&prometheus_registry.clone());

        metric_set_labels!(
            metrics,
            source_chain_rpc_url,
            ["Test Chain", 2, "localhost:8545"],
            1
        );
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
        let expected_body = "# HELP attestor_chain_key Attestor chain key\n# TYPE attestor_chain_key gauge\nattestor_chain_key 0\n# HELP source_chain_rpc_url Source chain node rpc url\n# TYPE source_chain_rpc_url gauge\nsource_chain_rpc_url{chain=\"Test Chain\",chain_key=\"2\",source_chain_rpc_url=\"localhost:8545\"} 1\n".to_string();

        assert_eq!(body_str, expected_body);
        assert!(body_str.contains("source_chain_rpc_url{chain=\"Test Chain\",chain_key=\"2\",source_chain_rpc_url=\"localhost:8545\"}"));
    }
}
