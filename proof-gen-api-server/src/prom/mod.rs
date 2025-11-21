use prometheus::{
    register_metrics, Error, Gauge, GaugeVec, HttpServer, Opts, PrometheusRegister, Registry,
};
use std::sync::Arc;

use crate::Config;

use crate::metric_set;

/// Starts the Prometheus metrics server and registers the prover metrics.
/// returns an optional `ProverMetrics` instance if registration is successful.
pub fn start_prom_server(config: &Config) -> Option<ProofGenServerMetrics> {
    let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
    let metrics: Option<ProofGenServerMetrics> = register_metrics(&prometheus_registry.clone());

    // set initial metrics
    metric_set!(metrics, gen_server_chain_key, &config.chain_key);

    // Create http server for metrics
    let http_server = Arc::new(HttpServer {
        prometheus_registry,
        bind_address: config.prometheus_host.clone(),
        port: config.prometheus_port,
    });
    tokio::spawn(http_server.clone().run_http_server());

    metrics
}

// TODO: Actually increment all of these where appropriate
#[allow(unused)]
#[derive(Clone, Debug)]
pub struct ProofGenServerMetrics {
    pub gen_server_chain_key: Gauge,
    pub proof_requests_received: GaugeVec,
    pub proof_requests_failed: GaugeVec,
    pub proof_requests_succeeded: GaugeVec,
    pub attestation_network_height: GaugeVec,
}

impl PrometheusRegister for ProofGenServerMetrics {
    const DESCRIPTION: &'static str = "proof_gen_server";
    fn register(registry: &Registry) -> Result<Self, Error> {
        let proof_gen_server_chain_key =
            Gauge::new("proof_gen_server_chain_key", "Proof gen server chain key")?;
        registry.register(Box::new(proof_gen_server_chain_key.clone()))?;

        let proof_requests_received = GaugeVec::new(
            Opts::new(
                "number_of_requests_received",
                "The number of proof requests received by the proof gen server",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(proof_requests_received.clone()))?;

        let proof_requests_failed = GaugeVec::new(
            Opts::new(
                "number_of_proof_generation_requests_failed",
                "The number of proof generation requests that have failed",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(proof_requests_failed.clone()))?;

        let proof_requests_succeeded = GaugeVec::new(
            Opts::new(
                "number_of_proof_generation_requests_successful",
                "The number of proof generation requests that have succeeded",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(proof_requests_succeeded.clone()))?;

        let attestation_network_height = GaugeVec::new(
            Opts::new(
                "attestation_network_height",
                "Current height of the attestation network",
            ),
            &["chain", "chain_key"],
        )?;
        registry.register(Box::new(attestation_network_height.clone()))?;

        Ok(Self {
            gen_server_chain_key: proof_gen_server_chain_key,
            proof_requests_received,
            proof_requests_failed,
            proof_requests_succeeded,
            attestation_network_height,
        })
    }
}

#[macro_export]
macro_rules! metric_inc_with_labels {
    ($metrics:expr, $m:ident, $chain_name:expr, $chain_key:expr) => {{
        if let Some(metrics) = $metrics.as_ref() {
            metrics
                .$m
                .with_label_values(&[&$chain_name.to_string(), &$chain_key.to_string()])
                .inc();
        }
    }};
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
macro_rules! metric_set_labels {
    ($metrics:expr, $m:ident, $metric_label_1:expr, $metric_label_2:expr, $v:expr) => {{
        let val: u64 = format!("{}", $v).parse().unwrap();

        if let Some(metrics) = $metrics.as_ref() {
            #[allow(clippy::cast_precision_loss)]
            metrics
                .$m
                .with_label_values(&[&$metric_label_1.to_string(), &$metric_label_2.to_string()])
                .set(val as f64);
        }
    }};
}

#[cfg(test)]
pub(crate) mod tests {
    use super::ProofGenServerMetrics;
    use axum::{
        body::{to_bytes, Body},
        http::{Request, StatusCode},
    };
    use prometheus::{register_metrics, HttpServer, Registry};
    use std::sync::Arc;

    use tower::util::ServiceExt;

    #[tokio::test]
    async fn prometheus_metrics_are_correctly_set_for_prover() {
        let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
        let metrics: Option<ProofGenServerMetrics> = register_metrics(&prometheus_registry.clone());

        metric_set!(metrics, gen_server_chain_key, 2);

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
        let expected_body = "# HELP prover_chain_key Prover chain key\n# TYPE prover_chain_key gauge\nprover_chain_key 2\n".to_string();

        assert_eq!(body_str, expected_body);
        assert_eq!(headers, expected_headers);
    }

    #[tokio::test]
    async fn prometheus_metrics_contain_correct_labels_when_set_for_prover() {
        let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
        let metrics: Option<ProofGenServerMetrics> = register_metrics(&prometheus_registry.clone());

        metric_inc_with_labels!(metrics, proof_requests_received, "Test Chain", 2);
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
        let expected_body = "# HELP proof_requests_received The number of proof requests received by the proof gen server\n# TYPE proof_requests_received gauge\nproof_requests_received{chain=\"Test Chain\",chain_key=\"2\"} 1\n# HELP proof_gen_server_chain_key Proof gen server chain key\n# TYPE proof_gen_server_chain_key gauge\nproof_gen_server_chain_key 2\n".to_string();

        assert_eq!(body_str, expected_body);
        assert!(body_str.contains("proof_requests_received{chain=\"Test Chain\",chain_key=\"2\"}"));
    }
}
