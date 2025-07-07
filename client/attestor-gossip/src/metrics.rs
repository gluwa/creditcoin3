use crate::LOG_TARGET;
use log::{debug, error};
use substrate_prometheus_endpoint::{
    register, Counter, CounterVec, GaugeVec, Opts, PrometheusError, Registry, U64,
};

/// Helper trait for registering attestor metrics to Prometheus registry.
pub(crate) trait PrometheusRegister<T: Sized = Self>: Sized {
    const DESCRIPTION: &'static str;
    fn register(registry: &Registry) -> Result<Self, PrometheusError>;
}

/// attestor voting-related metrics exposed through Prometheus
#[derive(Clone, Debug)]
pub struct VoterMetrics {
    pub attestor_votes_sent_per_chain: CounterVec<U64>,
    /// Best block finalized by attestor per chain
    pub attestor_best_block_per_chain: GaugeVec<U64>,
    /// Best block attestor voted on per chain
    pub attestor_best_voted_per_chain: GaugeVec<U64>,
    /// Number of times no Authority public key found in store
    pub _attestor_no_authority_found_in_store: Counter<U64>,
    /// Number of good votes successfully handled per chain
    pub attestor_good_votes_processed_per_chain: CounterVec<U64>,
    /// Number of equivocation votes received per chain
    pub attestor_equivocation_votes_per_chain: CounterVec<U64>,
    /// Number of invalid votes received
    pub attestor_invalid_votes: Counter<U64>,
    /// Number of valid votes successfully imported per chain
    pub attestor_imported_votes_per_chain: CounterVec<U64>,
    /// Number of attestor votes received from RPC per chain
    pub attestor_votes_from_rpc_per_chain: CounterVec<U64>,
    /// Number of attestor stale votes received per chain
    pub attestor_stale_votes_per_chain: CounterVec<U64>,
}

impl PrometheusRegister for VoterMetrics {
    const DESCRIPTION: &'static str = "voter";
    fn register(registry: &Registry) -> Result<Self, PrometheusError> {
        Ok(Self {
            attestor_votes_sent_per_chain: register(
                CounterVec::new(
                    Opts::new(
                        "substrate_attestor_votes_sent",
                        "Number of votes sent by this node per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
            attestor_best_block_per_chain: register(
                GaugeVec::new(
                    Opts::new(
                        "substrate_attestor_best_block",
                        "Best block finalized by attestor per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,

            attestor_best_voted_per_chain: register(
                GaugeVec::new(
                    Opts::new(
                        "substrate_attestor_best_voted",
                        "Best block voted on by attestor per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
            _attestor_no_authority_found_in_store: register(
                Counter::new(
                    "substrate_attestor_no_authority_found_in_store",
                    "Number of times no Authority public key found in store",
                )?,
                registry,
            )?,
            attestor_good_votes_processed_per_chain: register(
                CounterVec::new(
                    Opts::new(
                        "substrate_attestor_successful_handled_votes",
                        "Number of good votes successfully handled per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
            attestor_equivocation_votes_per_chain: register(
                CounterVec::new(
                    Opts::new(
                        "substrate_attestor_equivocation_votes",
                        "Number of equivocation votes received per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
            attestor_invalid_votes: register(
                Counter::new(
                    "substrate_attestor_invalid_votes",
                    "Number of invalid votes received",
                )?,
                registry,
            )?,
            attestor_imported_votes_per_chain: register(
                CounterVec::new(
                    Opts::new(
                        "attestor_imported_votes",
                        "Number of valid votes successfully imported per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
            attestor_votes_from_rpc_per_chain: register(
                CounterVec::new(
                    Opts::new(
                        "attestor_votes_from_rpc",
                        "Number of attestor votes received from RPC per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
            attestor_stale_votes_per_chain: register(
                CounterVec::new(
                    Opts::new(
                        "attestor_stale_votes",
                        "Number of attestor stale votes received per chain",
                    ),
                    &["chain_key"],
                )?,
                registry,
            )?,
        })
    }
}

pub(crate) fn register_metrics<T: PrometheusRegister>(
    prometheus_registry: Option<substrate_prometheus_endpoint::Registry>,
) -> Option<T> {
    prometheus_registry
        .as_ref()
        .map(T::register)
        .and_then(|result| match result {
            Ok(metrics) => {
                debug!(target: LOG_TARGET, "🥩 Registered {} metrics", T::DESCRIPTION);
                Some(metrics)
            }
            Err(err) => {
                error!(
                    target: LOG_TARGET,
                    "🥩 Failed to register {} metrics: {:?}",
                    T::DESCRIPTION,
                    err
                );
                None
            }
        })
}

// Note: we use the `format` macro to convert an expr into a `u64`. This will fail,
// if expr does not derive `Display`.
#[macro_export]
macro_rules! metric_set_chain {
    ($metrics:expr, $m:ident, $chain_key:expr, $v:expr) => {{
        let val: u64 = format!("{}", $v).parse().unwrap();

        if let Some(metrics) = $metrics.as_ref() {
            metrics
                .$m
                .with_label_values(&[&$chain_key.to_string()])
                .set(val);
        }
    }};
}

#[macro_export]
macro_rules! metric_inc_chain {
    ($metrics:expr, $m:ident, $chain_key:expr) => {{
        if let Some(metrics) = $metrics.as_ref() {
            metrics
                .$m
                .with_label_values(&[&$chain_key.to_string()])
                .inc();
        }
    }};
}

#[macro_export]
macro_rules! metric_inc {
    ($metrics:expr, $m:ident) => {{
        if let Some(metrics) = $metrics.as_ref() {
            metrics.$m.inc();
        }
    }};
}

#[macro_export]
macro_rules! metric_get {
    ($metrics:expr, $m:ident) => {{
        $metrics.as_ref().map(|metrics| metrics.$m.clone())
    }};
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn should_register_metrics() {
        let registry = Some(Registry::new());
        assert!(register_metrics::<VoterMetrics>(registry.clone()).is_some());
    }
}
