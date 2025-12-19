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
}

impl Metrics {
    pub fn new(config: Config) -> Self {
        let mut registry = prometheus_client::registry::Registry::default();
        let metrics_production = prometheus_client::metrics::family::Family::default();
        let metrics_lag = prometheus_client::metrics::family::Family::default();

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

        let metrics = Self {
            registry,
            metrics_production,
            metrics_lag,
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
}
