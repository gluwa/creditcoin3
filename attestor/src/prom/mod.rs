use prometheus::{Error, Gauge, GaugeVec, Opts, Registry};
use std::sync::Arc;
use tracing::{debug, error};

use crate::util::sanitize_url::sanitize_rpc_url_api_key;
use crate::Config;

pub mod http;

use crate::{metric_set, metric_set_labels};

/// Starts the Prometheus metrics server and registers the attestor metrics.
/// returns an optional `AttestorMetrics` instance if registration is successful.
pub fn start_prom_server(config: &Config) -> Option<AttestorMetrics> {
    let prometheus_registry: Arc<Registry> = Arc::new(Registry::new());
    let metrics: Option<AttestorMetrics> = register_metrics(&prometheus_registry.clone());

    // set initial metrics
    metric_set!(metrics, attestor_chain_key, &config.chain_key);
    metric_set_labels!(
        metrics,
        source_chain_rpc_url,
        sanitize_rpc_url_api_key(&config.eth_rpc_url),
        &config.chain_key,
        1
    );
    metric_set_labels!(
        metrics,
        cc3next_rpc_url,
        &config.cc3_rpc_url,
        &config.chain_key,
        1
    );
    // Create http server for metrics
    let http_server = Arc::new(http::HttpServer {
        prometheus_registry,
        bind_address: config.prometheus_host.clone(),
        port: config.prometheus_port,
    });
    tokio::spawn(http_server.clone().run_http_server());

    metrics
}

type PrometheusError = Error;
const LOG_TARGET: &str = "attestor";
/// Helper trait for registering attestor metrics to Prometheus registry.
pub(crate) trait PrometheusRegister<T: Sized = Self>: Sized {
    const DESCRIPTION: &'static str;
    fn register(registry: &Registry) -> Result<Self, PrometheusError>;
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
    fn register(registry: &Registry) -> Result<Self, PrometheusError> {
        let last_voted_for = GaugeVec::new(
            Opts::new(
                "last_block_voted_for",
                "The last block the attestor voted for",
            ),
            &["chain_key"],
        )?;
        registry.register(Box::new(last_voted_for.clone()))?;

        let last_finalized_attestation = GaugeVec::new(
            Opts::new(
                "last_finalized_attestation",
                "The last finalized attestation header",
            ),
            &["chain_key"],
        )?;
        registry.register(Box::new(last_finalized_attestation.clone()))?;

        let source_chain_height = GaugeVec::new(
            Opts::new(
                "source_chain_height",
                "The last finalized source chain header",
            ),
            &["chain_key"],
        )?;
        registry.register(Box::new(source_chain_height.clone()))?;

        let cc_current_epoch = GaugeVec::new(
            Opts::new("cc_current_epoch", "The current epoch of the cc chain"),
            &["chain_key"],
        )?;
        registry.register(Box::new(cc_current_epoch.clone()))?;

        let attestor_chain_key = Gauge::new("attestor_chain_key", "Attestor chain key")?;
        registry.register(Box::new(attestor_chain_key.clone()))?;

        let source_chain_rpc_url = GaugeVec::new(
            Opts::new("source_chain_rpc_url", "Source chain node rpc url"),
            &["source_chain_rpc_url", "chain_key"],
        )?;
        registry.register(Box::new(source_chain_rpc_url.clone()))?;

        let cc3next_rpc_url = GaugeVec::new(
            Opts::new("cc3next_rpc_url", "cc3next node rpc url"),
            &["cc3next_rpc_url", "chain_key"],
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

pub(crate) fn register_metrics<T: PrometheusRegister>(
    prometheus_registry: &Arc<prometheus::Registry>,
) -> Option<T> {
    match T::register(prometheus_registry) {
        Ok(metrics) => {
            debug!(target: LOG_TARGET, "📈 Registered {} metrics", T::DESCRIPTION);
            Some(metrics)
        }
        Err(err) => {
            error!(
                target: LOG_TARGET,
                "📈 Failed to register {} metrics: {:?}",
                T::DESCRIPTION,
                err
            );
            None
        }
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
    use super::*;

    #[test]
    fn should_register_metrics() {
        let registry = Arc::new(Registry::new());
        assert!(register_metrics::<AttestorMetrics>(&registry.clone()).is_some());
    }
}
