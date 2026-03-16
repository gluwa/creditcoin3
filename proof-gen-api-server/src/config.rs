#[derive(Debug, Clone)]
/// Server configuration
/// - `bind_host`: The IP address to bind the API server to (IPv4 or IPv6)
/// - `bind_port`: The port to bind the API server to
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account (optional, not needed for read-only operations)
/// - `chain_key`: Chain key for the source chain, must match the chain key on creditcoin3
/// - `eth_rpc_url`: Ethereum RPC url
/// - `redis_url`: Optional Redis URL for Ethereum block caching
/// - `redis_cluster_mode`: When true, use Redis Cluster client (required for Redis Cluster deployments)
/// - `indexer_url`: Optional CC3 Indexer GraphQL URL for pre-fetching continuity proofs
/// - `max_batch_size`: Maximum amount of concurrent futures spawned when generating proofs for batch requests or when extracting transaction indexes from transaction hashes. Adjust based on expected load and RPC rate limits.
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    pub cc3_rpc_url: String,
    pub cc3_key: Option<String>,
    pub chain_key: u64,
    pub eth_rpc_url: String,
    pub redis_url: Option<String>,
    pub redis_cluster_mode: bool,
    pub indexer_url: Option<String>,
    pub max_batch_size: usize,
    /// Optional archiver HTTP URL. When set, continuity proofs are built from
    /// pre-computed merkle roots served by the archiver instead of fetching
    /// full blocks from Ethereum RPC.
    pub archiver_url: Option<String>,
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
            redis_cluster_mode: false,
            indexer_url: None,
            max_batch_size: 10,
            archiver_url: None,
        }
    }
}
