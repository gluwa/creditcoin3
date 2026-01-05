use crate::prelude::*;

#[derive(attestor_macro::Builder)]
pub(crate) struct Config {
    name: String,
    address: cc_client::AccountId32,
    peer_id: libp2p::PeerId,
    chain_key: attestor_primitives::ChainKey,

    attestation_latest_eth: common::types::Height,
    attestation_latest_cc3: common::types::Height,
    attestation_interval: std::num::NonZero<common::types::Height>,
}

#[derive(Debug)]
pub(crate) struct Metrics {
    pub registry: prometheus_client::registry::Registry,

    pub metrics_production: prometheus_client::metrics::family::Family<
        labels::LabelAttestationProgress,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

    pub metrics_lag: prometheus_client::metrics::family::Family<
        labels::LabelAttestationChain,
        prometheus_client::metrics::gauge::Gauge,
    >,

    pub metrics_hardware: prometheus_client::metrics::family::Family<
        labels::LabelHardware,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

    pub metrics_delay: prometheus_client::metrics::family::Family<
        labels::LabelAttestationLifecycle,
        prometheus_client::metrics::histogram::Histogram,
    >,

    pub metrics_p2p: prometheus_client::metrics::family::Family<
        labels::LabelPeerToPeer,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

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

        metrics.update_attestation_lag_eth(
            0,
            config.attestation_latest_eth,
            config.attestation_interval,
        );
        metrics.update_attestation_lag_cc3(
            0,
            config.attestation_latest_cc3,
            config.attestation_interval,
        );

        metrics
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

    pub async fn update_hardware(&self) {
        // We initialize a new hardware interface on each call to avoid having to acquire a
        // blocking lock on a global resource due to mutable requirements on `refresh_specifics`.
        let specifics = sysinfo::RefreshKind::nothing()
            .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
            .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram());
        let mut sys = sysinfo::System::new_with_specifics(specifics);

        // NOTE: CPU USAGE
        //
        // From the sysinfo docs: "Please note that the result [of calling global_cpu_usage] will
        // very likely be inaccurate at the first call. You need to call [refresh_cpu_usage] at
        // least twice (with a bit of time between each call, like 200 ms, take a look at
        // MINIMUM_CPU_UPDATE_INTERVAL for more information) to get accurate value as it uses
        // previous results to compute the next value."
        tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
        sys.refresh_specifics(specifics);

        let cpu_global = sys.global_cpu_usage();
        let cpu_count = sys.cpus().len() as f32;
        let usage_cpu = (cpu_global / cpu_count) as u64;

        let total_memory = sys.total_memory();
        let used_memory = sys.used_memory();
        let usage_memory = (used_memory / total_memory) * 100;

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
                peer_to_peer: labels::PeerToPeer::Count,
            })
            .inc();
    }

    pub fn decrease_peer_count(&self) {
        self.metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::Count,
            })
            .dec();
    }

    pub fn increase_gossipsub_message_count(&self) {
        self.metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::GossipsubMessage,
            })
            .inc();
    }

    pub fn increase_invalid_attestation_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::InvalidAttestation,
            })
            .inc();
    }

    pub fn increase_equivocation_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::Equivocation,
            })
            .inc();
    }

    pub fn increase_invalid_gossipsub_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::GossipsubMessage,
            })
            .inc();
    }

    pub fn increase_connection_failure_count(&self) {
        self.metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::ConnectionFailure,
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
        Count,
        GossipsubMessage,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelPeerToPeer {
        pub peer_to_peer: PeerToPeer,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum FailedState {
        InvalidAttestation,
        Equivocation,
        GossipsubMessage,
        ConnectionFailure,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelFailedState {
        pub failed_state: FailedState,
    }
}
