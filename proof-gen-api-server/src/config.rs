#[derive(Debug, Clone)]
/// Server configuration
/// - `bind_host`: The IP address to bind the API server to (IPv4 or IPv6)
/// - `bind_port`: The port to bind the API server to
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account (optional, not needed for read-only operations)
/// - `chain_key`: Chain key for the source chain, must match the chain key on creditcoin3
/// - `eth_rpc_url`: Ethereum RPC url
/// - `enable_prometheus_metrics`:
/// - `prometheus_host`:
/// - `prometheus_port`:
/// - `redis_url`: Optional Redis URL for Ethereum block caching
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    pub cc3_rpc_url: String,
    pub cc3_key: Option<String>,
    pub chain_key: u64,
    pub eth_rpc_url: String,
    pub enable_prometheus_metrics: bool,
    pub prometheus_host: String,
    pub prometheus_port: u16,
    pub redis_url: Option<String>,
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
            enable_prometheus_metrics: false,
            prometheus_host: "127.0.0.1".to_string(),
            prometheus_port: 9090,
            redis_url: None,
        }
    }
}
