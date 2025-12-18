#[derive(attestor_macro::Builder)]
pub(crate) struct Config {
    name: String,
    address: cc_client::AccountId32,
    peer_id: libp2p::PeerId,
    chain_key: attestor_primitives::ChainKey,
}

pub(crate) struct MetricsStore {
    pub registry: prometheus_client::registry::Registry,
}

impl MetricsStore {
    pub fn new(config: Config) -> Self {
        let mut registry = prometheus_client::registry::Registry::default();

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

        Self { registry }
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
