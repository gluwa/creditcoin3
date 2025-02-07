use crate::LOG_TARGET;
use log::{debug, error};
use substrate_prometheus_endpoint::{register, Counter, Gauge, PrometheusError, Registry, U64};

/// Helper trait for registering attestor metrics to Prometheus registry.
pub(crate) trait PrometheusRegister<T: Sized = Self>: Sized {
    const DESCRIPTION: &'static str;
    fn register(registry: &Registry) -> Result<Self, PrometheusError>;
}

/// attestor voting-related metrics exposed through Prometheus
#[derive(Clone, Debug)]
pub struct VoterMetrics {
    pub attestor_votes_sent: Counter<U64>,
    /// Best block finalized by attestor
    pub attestor_best_block: Gauge<U64>,
    /// Best block attestor voted on
    pub attestor_best_voted: Gauge<U64>,
    /// Number of times no Authority public key found in store
    pub attestor_no_authority_found_in_store: Counter<U64>,
    /// Number of good votes successfully handled
    pub attestor_good_votes_processed: Counter<U64>,
    /// Number of equivocation votes received
    pub attestor_equivocation_votes: Counter<U64>,
    /// Number of invalid votes received
    pub attestor_invalid_votes: Counter<U64>,
    /// Number of valid votes successfully imported
    pub attestor_imported_votes: Counter<U64>,
    /// Number of attestor votes received from RPC
    pub attestor_votes_from_rpc: Counter<U64>,
}

impl PrometheusRegister for VoterMetrics {
    const DESCRIPTION: &'static str = "voter";
    fn register(registry: &Registry) -> Result<Self, PrometheusError> {
        Ok(Self {
            attestor_votes_sent: register(
                Counter::new(
                    "substrate_attestor_votes_sent",
                    "Number of votes sent by this node",
                )?,
                registry,
            )?,
            attestor_best_block: register(
                Gauge::new(
                    "substrate_attestor_best_block",
                    "Best block finalized by attestor",
                )?,
                registry,
            )?,
            attestor_best_voted: register(
                Gauge::new(
                    "substrate_attestor_best_voted",
                    "Best block voted on by attestor",
                )?,
                registry,
            )?,
            attestor_no_authority_found_in_store: register(
                Counter::new(
                    "substrate_attestor_no_authority_found_in_store",
                    "Number of times no Authority public key found in store",
                )?,
                registry,
            )?,
            attestor_good_votes_processed: register(
                Counter::new(
                    "substrate_attestor_successful_handled_votes",
                    "Number of good votes successfully handled",
                )?,
                registry,
            )?,
            attestor_equivocation_votes: register(
                Counter::new(
                    "substrate_attestor_equivocation_votes",
                    "Number of equivocation votes received",
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
            attestor_imported_votes: register(
                Counter::new(
                    "attestor_imported_votes",
                    "Number of valid votes successfully imported",
                )?,
                registry,
            )?,
            attestor_votes_from_rpc: register(
                Counter::new(
                    "attestor_votes_from_rpc",
                    "Number of attestor votes received from RPC",
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
macro_rules! metric_set {
    ($metrics:expr, $m:ident, $v:expr) => {{
        let val: u64 = format!("{}", $v).parse().unwrap();

        if let Some(metrics) = $metrics.as_ref() {
            metrics.$m.set(val);
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
