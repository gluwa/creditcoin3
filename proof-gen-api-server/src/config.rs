#[derive(Debug, Clone)]
/// Server configuration
/// - `bind_host`: The IP address to bind the API server to (IPv4 or IPv6)
/// - `bind_port`: The port to bind the API server to
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account (optional, not needed for read-only operations)
/// - `chain_key`: Chain key for the source chain, must match the chain key on creditcoin3
/// - `eth_rpc_url`: Ethereum RPC url
/// - `redis_url`: Optional Redis URL for Ethereum block caching
/// - `indexer_url`: Optional CC3 Indexer GraphQL URL for pre-fetching continuity proofs
/// - `enable_prometheus_metrics`: Whether to enable Prometheus metrics
/// - `prometheus_host`: Host for Prometheus metrics endpoint
/// - `prometheus_port`: Port for Prometheus metrics endpoint
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    pub cc3_rpc_url: String,
    pub cc3_key: Option<String>,
    pub chain_key: u64,
    pub eth_rpc_url: String,
    pub redis_url: Option<String>,
    pub indexer_url: Option<String>,
    pub enable_prometheus_metrics: bool,
    pub prometheus_host: String,
    pub prometheus_port: u16,
}

impl Config {
    /// Convenience constructor for tests
    /// Builds a config with stable dummy values and does not read environment variables.
    /// - Uses loopback addresses for bind and metrics.
    /// - Accepts a `chain_key` parameter to match test expectations.
    pub fn new_mock_config(chain_key: u64) -> Self {
        Self {
            bind_host: "127.0.0.1".to_string(),
            bind_port: 3000,
            cc3_rpc_url: "ws://mock".to_string(),
            cc3_key: None,
            chain_key,
            eth_rpc_url: "http://mock".to_string(),
            redis_url: None,
            indexer_url: None,
            enable_prometheus_metrics: false,
            prometheus_host: "127.0.0.1".to_string(),
            prometheus_port: 9100,
        }
    }
}
