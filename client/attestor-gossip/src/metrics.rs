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
	/// Next block attestor should vote on
	pub attestor_should_vote_on: Gauge<U64>,
	/// Number of times no Authority public key found in store
	pub attestor_no_authority_found_in_store: Counter<U64>,
	/// Number of good votes successfully handled
	pub attestor_good_votes_processed: Counter<U64>,
	/// Number of equivocation votes received
	pub attestor_equivocation_votes: Counter<U64>,
	/// Number of invalid votes received
	pub attestor_invalid_votes: Counter<U64>,
	/// Number of valid but stale votes received
	pub attestor_stale_votes: Counter<U64>,
    /// Number of valid votes successfully imported
    pub attestor_imported_votes: Counter<U64>,
}

impl PrometheusRegister for VoterMetrics {
	const DESCRIPTION: &'static str = "voter";
	fn register(registry: &Registry) -> Result<Self, PrometheusError> {
		Ok(Self {
			attestor_votes_sent: register(
				Counter::new("substrate_attestor_votes_sent", "Number of votes sent by this node")?,
				registry,
			)?,
			attestor_best_block: register(
				Gauge::new("substrate_attestor_best_block", "Best block finalized by attestor")?,
				registry,
			)?,
			attestor_best_voted: register(
				Gauge::new("substrate_attestor_best_voted", "Best block voted on by attestor")?,
				registry,
			)?,
			attestor_should_vote_on: register(
				Gauge::new("substrate_attestor_should_vote_on", "Next block, attestor should vote on")?,
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
				Counter::new("substrate_attestor_invalid_votes", "Number of invalid votes received")?,
				registry,
			)?,
			attestor_stale_votes: register(
				Counter::new(
					"substrate_attestor_stale_votes",
					"Number of valid but stale votes received",
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
		})
	}
}

/// attestor block-import-related metrics exposed through Prometheus
#[derive(Clone, Debug)]
pub struct BlockImportMetrics {
	/// Number of Good Justification imports
	pub attestor_good_justification_imports: Counter<U64>,
	/// Number of Bad Justification imports
	pub attestor_bad_justification_imports: Counter<U64>,
}

impl PrometheusRegister for BlockImportMetrics {
	const DESCRIPTION: &'static str = "block-import";
	fn register(registry: &Registry) -> Result<Self, PrometheusError> {
		Ok(Self {
			attestor_good_justification_imports: register(
				Counter::new(
					"substrate_attestor_good_justification_imports",
					"Number of good justifications on block-import",
				)?,
				registry,
			)?,
			attestor_bad_justification_imports: register(
				Counter::new(
					"substrate_attestor_bad_justification_imports",
					"Number of bad justifications on block-import",
				)?,
				registry,
			)?,
		})
	}
}

/// attestor on-demand-justifications-related metrics exposed through Prometheus
#[derive(Clone, Debug)]
pub struct OnDemandIncomingRequestsMetrics {
	/// Number of Successful Justification responses
	pub attestor_successful_justification_responses: Counter<U64>,
	/// Number of Failed Justification responses
	pub attestor_failed_justification_responses: Counter<U64>,
}

impl PrometheusRegister for OnDemandIncomingRequestsMetrics {
	const DESCRIPTION: &'static str = "on-demand incoming justification requests";
	fn register(registry: &Registry) -> Result<Self, PrometheusError> {
		Ok(Self {
			attestor_successful_justification_responses: register(
				Counter::new(
					"substrate_attestor_successful_justification_responses",
					"Number of Successful Justification responses",
				)?,
				registry,
			)?,
			attestor_failed_justification_responses: register(
				Counter::new(
					"substrate_attestor_failed_justification_responses",
					"Number of Failed Justification responses",
				)?,
				registry,
			)?,
		})
	}
}

/// attestor on-demand-justifications-related metrics exposed through Prometheus
#[derive(Clone, Debug)]
pub struct OnDemandOutgoingRequestsMetrics {
	/// Number of times there was no good peer to request justification from
	pub attestor_on_demand_justification_no_peer_to_request_from: Counter<U64>,
	/// Number of on-demand justification peer refused valid requests
	pub attestor_on_demand_justification_peer_refused: Counter<U64>,
	/// Number of on-demand justification peer error
	pub attestor_on_demand_justification_peer_error: Counter<U64>,
	/// Number of on-demand justification invalid proof
	pub attestor_on_demand_justification_invalid_proof: Counter<U64>,
	/// Number of on-demand justification good proof
	pub attestor_on_demand_justification_good_proof: Counter<U64>,
	/// Number of live attestor peers available for requests.
	pub attestor_on_demand_live_peers: Gauge<U64>,
}

impl PrometheusRegister for OnDemandOutgoingRequestsMetrics {
	const DESCRIPTION: &'static str = "on-demand outgoing justification requests";
	fn register(registry: &Registry) -> Result<Self, PrometheusError> {
		Ok(Self {
			attestor_on_demand_justification_no_peer_to_request_from: register(
				Counter::new(
					"substrate_attestor_on_demand_justification_no_peer_to_request_from",
					"Number of times there was no good peer to request justification from",
				)?,
				registry,
			)?,
			attestor_on_demand_justification_peer_refused: register(
				Counter::new(
					"attestor_on_demand_justification_peer_refused",
					"Number of on-demand justification peer refused valid requests",
				)?,
				registry,
			)?,
			attestor_on_demand_justification_peer_error: register(
				Counter::new(
					"substrate_attestor_on_demand_justification_peer_error",
					"Number of on-demand justification peer error",
				)?,
				registry,
			)?,
			attestor_on_demand_justification_invalid_proof: register(
				Counter::new(
					"substrate_attestor_on_demand_justification_invalid_proof",
					"Number of on-demand justification invalid proof",
				)?,
				registry,
			)?,
			attestor_on_demand_justification_good_proof: register(
				Counter::new(
					"substrate_attestor_on_demand_justification_good_proof",
					"Number of on-demand justification good proof",
				)?,
				registry,
			)?,
			attestor_on_demand_live_peers: register(
				Gauge::new(
					"substrate_attestor_on_demand_live_peers",
					"Number of live attestor peers available for requests.",
				)?,
				registry,
			)?,
		})
	}
}

pub(crate) fn register_metrics<T: PrometheusRegister>(
	prometheus_registry: Option<substrate_prometheus_endpoint::Registry>,
) -> Option<T> {
	prometheus_registry.as_ref().map(T::register).and_then(|result| match result {
		Ok(metrics) => {
			debug!(target: LOG_TARGET, "🥩 Registered {} metrics", T::DESCRIPTION);
			Some(metrics)
		},
		Err(err) => {
			error!(
				target: LOG_TARGET,
				"🥩 Failed to register {} metrics: {:?}",
				T::DESCRIPTION,
				err
			);
			None
		},
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
		assert!(register_metrics::<BlockImportMetrics>(registry.clone()).is_some());
		assert!(register_metrics::<OnDemandIncomingRequestsMetrics>(registry.clone()).is_some());
		assert!(register_metrics::<OnDemandOutgoingRequestsMetrics>(registry.clone()).is_some());
	}
}
