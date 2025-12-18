use crate::prelude::*;

#[derive(attestor_macro::Builder)]
pub(crate) struct Config {
    name: String,
    address: cc_client::AccountId32,
    // peer_id: libp2p::PeerId,
    chain_key: attestor_primitives::ChainKey,

    attestation_latest_cc3: common::types::Height,
}

pub(crate) struct Metrics {
    pub registry: prometheus_client::registry::Registry,

    pub production: prometheus_client::metrics::family::Family<
        labels::LabelAttestationProgress,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,
}

impl Metrics {
    pub fn new(config: Config) -> Self {
        let mut registry = prometheus_client::registry::Registry::default();
        let production = prometheus_client::metrics::family::Family::default();

        registry.register(
            "attestor",
            "Basic operational information",
            prometheus_client::metrics::info::Info::new(items::MetricsInfo {
                name: config.name,
                address: config.address.to_string(),
                // peer_id: config.peer_id.to_string(),
                chain_key: config.chain_key,
            }),
        );

        registry.register(
            "production",
            "Progress in attestation production and finalization",
            production.clone(),
        );

        let mut metrics = Self {
            registry,
            production,
        };

        metrics.set_attestation_local(config.attestation_latest_cc3);
        metrics.set_attestation_finalized(config.attestation_latest_cc3);

        metrics
    }

    pub fn set_attestation_local(&mut self, height: common::types::Height) {
        self.production
            .get_or_create(&labels::LabelAttestationProgress {
                progress: labels::AttestationProgress::Local,
            })
            .set(height);
    }

    pub fn set_attestation_finalized(&mut self, height: common::types::Height) {
        self.production
            .get_or_create(&labels::LabelAttestationProgress {
                progress: labels::AttestationProgress::Finalized,
            })
            .set(height);
    }
}

mod items {
    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct MetricsInfo {
        pub name: String,
        pub address: String,
        // pub peer_id: String,
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
}
