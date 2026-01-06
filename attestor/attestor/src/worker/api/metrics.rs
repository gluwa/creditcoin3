//! Attestor [prometheus] metrics, see [`Metrics`] for a list of available metrics.
//!
//! [prometheus]:  prometheus_client

use crate::prelude::*;

#[derive(attestor_macro::Builder)]
pub struct Config {
    name: String,
    address: cc_client::AccountId32,
    peer_id: libp2p::PeerId,
    chain_key: attestor_primitives::ChainKey,
    start_height: common::types::Height,

    attestation_latest_eth: common::types::Height,
    attestation_latest_cc3: common::types::Height,
    attestation_interval: std::num::NonZero<common::types::Height>,
}

/// Global atomic metrics store.
///
/// # Metrics
///
/// - [hardware]: basic hardware metrics.
/// - [production]: keeps track of attestation production relative to the execution chain.
/// - [lag]: monitors an attestor’s advancement relative to the source chain and the execution chain.
/// - [delay]: aggregates the elapsed time throughout an attestation’s life cycle.
/// - [p2p]: monitors p2p network health.
/// - [errors]: counts failed state.
///
/// [hardware]: Self::metrics_hardware
/// [production]: Self::metrics_production
/// [lag]: Self::metrics_lag
/// [delay]: Self::metrics_delay
/// [p2p]: Self::metrics_p2p
/// [errors]: Self::metrics_error
#[derive(Debug)]
pub struct Metrics {
    registry: prometheus_client::registry::Registry,

    /// Basic hardware metrics.
    ///
    /// - _CPU usage_ (via [`global_cpu_usage`])
    /// - _RAM usage_ (via [`used_memory`])
    ///
    /// See [`update_hardware`] for implementation details.
    ///
    /// [`global_cpu_usage`]: sysinfo::System::global_cpu_usage
    /// [`used_memory`]: sysinfo::System::used_memory
    /// [`update_hardware`]: Self::update_hardware
    pub metrics_hardware: prometheus_client::metrics::family::Family<
        labels::LabelHardware,
        prometheus_client::metrics::gauge::Gauge<f64, std::sync::atomic::AtomicU64>,
    >,

    /// Metrics which keep track of attestation production relative to the execution chain.
    ///
    /// - _Latest locally produced attestation height_ ([`Gauge`])
    /// - _Latest finalized attestation height_ ([`Gauge`])
    ///
    /// Finalization data is already encapsulated by the [lag] metrics, so these are more for
    /// debugging and local observability.
    ///
    /// See [`set_attestation_local`] and [`set_attestation_finalized`] for implementation details.
    ///
    /// [`Gauge`]: prometheus_client::metrics::gauge::Gauge
    /// [lag]: Self::metrics_lag
    /// [`set_attestation_local`]: Self::set_attestation_local
    /// [`set_attestation_finalized`]: Self::set_attestation_finalized
    pub metrics_production: prometheus_client::metrics::family::Family<
        labels::LabelAttestationProgress,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

    /// Metrics which keep track of an attestor’s advancement relative to the source chain and the
    /// execution chain. These count the number of attestations by which the attestor is ahead (if
    /// positive) or behind (if negative) for each chain.
    ///
    /// - _Attestation source chain lag_ ([`Gauge`])
    /// - _Attestation execution chain lag_ ([`Gauge`])
    ///
    /// See [`update_attestation_lag_eth`] and [`update_attestation_lag_cc3`] for implementation
    /// details.
    ///
    /// ✅ A **positive value** for the execution chain indicates that the attestor is able to
    /// keep ahead of finality.
    ///
    /// ⚠️ A **positive value** for the source chain indicates that the source chain has stalled.
    ///
    /// ⚠️ A **negative value** for the source chain indicates we are not producing attestations
    /// fast enough.
    ///
    /// ⚠️ A **negative value** for the execution chain indicates we are not receiving/validating
    /// attestations fast enough.
    ///
    /// ❌ A **large positive value** for the execution chain indicates the runtime is being
    /// overwhelmed.
    ///
    /// [`Gauge`]: prometheus_client::metrics::gauge::Gauge
    /// [`update_attestation_lag_eth`]: Self::update_attestation_lag_eth
    /// [`update_attestation_lag_cc3`]: Self::update_attestation_lag_cc3
    pub metrics_lag: prometheus_client::metrics::family::Family<
        labels::LabelAttestationChain,
        prometheus_client::metrics::gauge::Gauge,
    >,

    /// Metrics which keep track of elapsed time throughout an attestation’s lifecycle.
    ///
    /// - _Attestation production delay_ ([`Histogram`])
    /// - _Attestation quorum delay_ ([`Histogram`])
    /// - _Attestation finalization delay_ ([`Histogram`])
    ///
    /// See [`update_attestation_delay_production`], [`update_attestation_delay_quorum`] and
    /// [`update_attestation_delay_finalization`] for implementation details.
    ///
    /// ✅ Values **converging to a low time interval** indicates all is well.
    ///
    /// ⚠️ **Outliers in attestation production** indicate we are hashing either much larger or much
    /// smaller source chain blocks.
    ///
    /// ⚠️ **Outliers in quorum delay** indicate the attestation network is either under strain, or
    /// recovering from strain.
    ///
    /// ❌ **Outliers in finalization** indicate the attestation network is stalling, or recovering
    /// from a stall.
    ///
    /// [`Histogram`]: prometheus_client::metrics::histogram::Histogram
    /// [`update_attestation_delay_production`]: Self::update_attestation_delay_production
    /// [`update_attestation_delay_quorum`]: Self::update_attestation_delay_quorum
    /// [`update_attestation_delay_finalization`]: Self::update_attestation_delay_finalization
    pub metrics_delay: prometheus_client::metrics::family::Family<
        labels::LabelAttestationLifecycle,
        prometheus_client::metrics::histogram::Histogram,
    >,

    /// Metrics which keep track of the attestor p2p network’s health.
    ///
    /// - _Active peer count_ ([`Gauge`])
    /// - _Gossipsub messages_ ([`Gauge`])
    ///
    /// The gossipsub message count is meant to be interpreted as the change in frequency of
    /// gossipsub messages via the PromQL [`rate`] function so as to analyze variance in network
    /// traffic.
    ///
    /// See [`increase_peer_count`], [`decrease_peer_count`] and
    /// [`increase_gossipsub_message_count`] for implementation details.
    ///
    /// ✅ An active peer count **above the quorum size** indicates all is well.
    ///
    /// ✅ A **steady rate** of gossipsub messages indicates votes are being correctly broadcasted
    /// and network liveness is maintained.
    ///
    /// ⚠️ An active peer count **at the quorum size** indicate a node might stop reaching quorum if
    /// any peer goes down.
    ///
    /// ⚠️ An **decreasing rate** of gossipsub messages indicate the attestation network is under
    /// strain.
    ///
    /// ⚠️ An **increasing rate** of gossipsub messages indicates the attestation network is
    /// recovering from strain.
    ///
    /// ❌ An active peer count **under the quorum size** indicates things are very, very wrong!
    /// Peers are not able to gossip votes and quorum will never be reached! This indicates a failed
    /// update to the networking policy or that valid nodes have been taken down!
    ///
    /// [`Gauge`]: prometheus_client::metrics::gauge::Gauge
    /// [`rate`]: https://prometheus.io/docs/prometheus/latest/querying/functions/#rate
    /// [`increase_peer_count`]: Self::increase_peer_count
    /// [`decrease_peer_count`]: Self::decrease_peer_count
    /// [`increase_gossipsub_message_count`]: Self::increase_gossipsub_message_count
    pub metrics_p2p: prometheus_client::metrics::family::Family<
        labels::LabelPeerToPeer,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

    /// Metrics which keep track of failed state.
    ///
    /// - _Known invalid attestations_ ([`Counter`])
    /// - _Know equivocations_ ([`Counter`])
    /// - _Invalid gossipsub messages_ ([`Counter`])
    /// - _Failed connections_ ([`Counter`])
    ///
    /// The failed connection count is meant to be interpreted as the rate of failed p2p handshakes
    /// via the PromQL [`rate`] function.
    ///
    /// ✅ Invalid attestations, invalid message and equivocations being **zero** indicates that all
    /// is well.
    ///
    /// ✅ **Small periodic failures in connection is not an issue**. Gossipsub periodically
    /// refreshes its peers to keep its peerset up to date and protect against eclipse attacks. As
    /// part of this, the protocol might attempt to handshake with incompatible peers if they are
    /// discoverable in the local network (this is the case for example if incompatible nodes enable
    /// MDns discovery).
    ///
    /// ⚠️ **Large, repeated and continuous spikes in failed connection is an issue** as
    /// it indicates either a bug in the attestor code or of an attack.
    ///
    /// ❌ Invalid attestations, invalid messages or equivocations being **greater than zero**
    /// indicates either a critical bug in the attestor code or that we are under attack.
    ///
    /// [`Counter`]: prometheus_client::metrics::counter::Counter
    /// [`rate`]: https://prometheus.io/docs/prometheus/latest/querying/functions/#rate
    pub metrics_error: prometheus_client::metrics::family::Family<
        labels::LabelFailedState,
        prometheus_client::metrics::counter::Counter<u64, std::sync::atomic::AtomicU64>,
    >,
}

impl Metrics {
    pub fn new(config: Config) -> Self {
        let mut registry = prometheus_client::registry::Registry::default();
        let metrics_production = prometheus_client::metrics::family::Family::default();
        let metrics_lag = prometheus_client::metrics::family::Family::default();
        let metrics_hardware = prometheus_client::metrics::family::Family::default();
        let metrics_delay = prometheus_client::metrics::family::Family::<
            labels::LabelAttestationLifecycle,
            _,
        >::new_with_constructor(|| {
            prometheus_client::metrics::histogram::Histogram::new(
                prometheus_client::metrics::histogram::exponential_buckets(0.01, 2.0, 15),
            )
        });
        let metrics_p2p = prometheus_client::metrics::family::Family::default();
        let metrics_error = prometheus_client::metrics::family::Family::default();

        registry.register(
            "attestor",
            "Basic operational information",
            prometheus_client::metrics::info::Info::new(items::MetricsInfo {
                name: config.name,
                address: config.address.to_string(),
                peer_id: config.peer_id.to_string(),
                chain_key: config.chain_key,
            }),
        );

        registry.register(
            "production",
            "Progress in attestation production and finalization",
            metrics_production.clone(),
        );

        registry.register(
            "lag",
            "Lag in attestation production, per chain",
            metrics_lag.clone(),
        );

        registry.register(
            "hardware",
            "Hardware usage metrics",
            metrics_hardware.clone(),
        );

        registry.register(
            "attestation_delay",
            "Attestation processing delay per lifecycle stage",
            metrics_delay.clone(),
        );

        registry.register(
            "peer_to_peer",
            "Peer-to-peer networking metrics",
            metrics_p2p.clone(),
        );

        registry.register(
            "failed_states",
            "Counts of various failure states",
            metrics_error.clone(),
        );

        let metrics = Self {
            registry,
            metrics_production,
            metrics_lag,
            metrics_hardware,
            metrics_delay,
            metrics_p2p,
            metrics_error,
        };

        metrics.set_attestation_local(config.attestation_latest_cc3);
        metrics.set_attestation_finalized(config.attestation_latest_cc3);

        let attestation_local = config
            .start_height
            .saturating_sub(config.attestation_interval.get());
        metrics.update_attestation_lag_eth(
            attestation_local,
            config.attestation_latest_eth,
            config.attestation_interval,
        );
        metrics.update_attestation_lag_cc3(
            attestation_local,
            config.attestation_latest_cc3,
            config.attestation_interval,
        );

        metrics
    }

    pub fn encode(&self) -> String {
        let mut buffer = String::new();
        prometheus_client::encoding::text::encode(&mut buffer, &self.registry).unwrap();
        buffer
    }

    pub async fn update_hardware(&self) {
        if let Ok(pid) = sysinfo::get_current_pid() {
            // We initialize a new hardware interface on each call to avoid having to acquire a
            // blocking lock on a global resource due to mutable requirements on
            // `refresh_specifics`.
            let specifics = sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram())
                .with_processes(
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory(),
                );
            let mut sys = sysinfo::System::new_with_specifics(specifics);

            // NOTE: CPU USAGE
            //
            // From the sysinfo docs: "Please note that the result [of calling global_cpu_usage]
            // will very likely be inaccurate at the first call. You need to call
            // [refresh_cpu_usage] at least twice (with a bit of time between each call, like 200
            // ms, take a look at MINIMUM_CPU_UPDATE_INTERVAL for more information) to get accurate
            // value as it uses previous results to compute the next value."
            tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
            sys.refresh_specifics(specifics);

            // NOTE: METRICS
            //
            // Methods like `global_cpu_usage` target system-wide hardware metrics -we need to make
            // sure we retrieve hardware data for the current process only.
            if let Some(process) = sys.process(pid) {
                let cpu_process = process.cpu_usage() as f64;
                let cpu_count = sys.cpus().len() as f64;
                let usage_cpu = cpu_process / cpu_count;

                let memory_process = process.memory() as f64;
                let memory_total = sys.total_memory() as f64;
                let usage_memory = (memory_process / memory_total) * 100.0;

                self.metrics_hardware
                    .get_or_create(&labels::LabelHardware {
                        hardware: labels::Hardware::Cpu,
                    })
                    .set(usage_cpu);
                self.metrics_hardware
                    .get_or_create(&labels::LabelHardware {
                        hardware: labels::Hardware::Memory,
                    })
                    .set(usage_memory);
            }
        };
    }

    pub fn set_attestation_local(&self, height: common::types::Height) {
        self.metrics_production
            .get_or_create(&labels::LabelAttestationProgress {
                progress: labels::AttestationProgress::Local,
            })
            .set(height);
    }

    pub fn set_attestation_finalized(&self, height: common::types::Height) {
        self.metrics_production
            .get_or_create(&labels::LabelAttestationProgress {
                progress: labels::AttestationProgress::Finalized,
            })
            .set(height);
    }

    pub fn update_attestation_lag_eth(
        &self,
        attestation_local: common::types::Height,
        block_latest_eth: common::types::Height,
        interval: std::num::NonZero<common::types::Height>,
    ) {
        let attestation_local = attestation_local as i64;
        let attestation_latest_eth = block_latest_eth as i64;
        let interval = interval.get() as i64;
        let lag_eth = attestation_local.saturating_sub(attestation_latest_eth) / interval;

        self.metrics_lag
            .get_or_create(&labels::LabelAttestationChain {
                chain: labels::AttestationChain::Eth,
            })
            .set(lag_eth);
    }

    pub fn update_attestation_lag_cc3(
        &self,
        attestation_local: common::types::Height,
        attestation_latest_cc3: common::types::Height,
        interval: std::num::NonZero<common::types::Height>,
    ) {
        use prometheus_client::metrics::gauge::Atomic as _;

        let attestation_local = attestation_local as i64;
        let attestation_latest_cc3 = attestation_latest_cc3 as i64;
        let interval = interval.get() as i64;
        let lag_cc3 = attestation_local.saturating_sub(attestation_latest_cc3) / interval;

        self.metrics_lag
            .get_or_create(&labels::LabelAttestationChain {
                chain: labels::AttestationChain::CC3,
            })
            .inner()
            .set(lag_cc3);
    }

    pub fn update_attestation_delay_production(&self, delay: f64) {
        self.metrics_delay
            .get_or_create(&labels::LabelAttestationLifecycle {
                lifecycle: labels::AttestationLifecycle::Production,
            })
            .observe(delay);
    }

    pub fn update_attestation_delay_quorum(&self, delay: f64) {
        self.metrics_delay
            .get_or_create(&labels::LabelAttestationLifecycle {
                lifecycle: labels::AttestationLifecycle::Quorum,
            })
            .observe(delay);
    }

    pub fn update_attestation_delay_finalization(&self, delay: f64) {
        self.metrics_delay
            .get_or_create(&labels::LabelAttestationLifecycle {
                lifecycle: labels::AttestationLifecycle::Finalization,
            })
            .observe(delay);
    }

    pub fn increase_peer_count(&self) {
        self.metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::Peers,
            })
            .inc();
    }

    pub fn decrease_peer_count(&self) {
        self.metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::Peers,
            })
            .dec();
    }

    pub fn increase_gossipsub_message_count(&self) {
        self.metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::GossipsubMessages,
            })
            .inc();
    }

    pub fn increase_invalid_attestation_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::InvalidAttestations,
            })
            .inc();
    }

    pub fn increase_equivocation_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::Equivocations,
            })
            .inc();
    }

    pub fn increase_invalid_gossipsub_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::GossipsubMessages,
            })
            .inc();
    }

    pub fn increase_connection_failure_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::ConnectionFailures,
            })
            .inc();
    }
}

mod items {
    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct MetricsInfo {
        pub name: String,
        pub address: String,
        pub peer_id: String,
        pub chain_key: attestor_primitives::ChainKey,
    }
}

mod labels {
    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum AttestationProgress {
        Local,
        Finalized,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelAttestationProgress {
        pub progress: AttestationProgress,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum AttestationChain {
        Eth,
        CC3,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelAttestationChain {
        pub chain: AttestationChain,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum Hardware {
        Cpu,
        Memory,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelHardware {
        pub hardware: Hardware,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum AttestationLifecycle {
        Production,
        Quorum,
        Finalization,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelAttestationLifecycle {
        pub lifecycle: AttestationLifecycle,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum PeerToPeer {
        Peers,
        GossipsubMessages,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelPeerToPeer {
        pub peer_to_peer: PeerToPeer,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum FailedState {
        InvalidAttestations,
        Equivocations,
        GossipsubMessages,
        ConnectionFailures,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelFailedState {
        pub failed_state: FailedState,
    }
}
