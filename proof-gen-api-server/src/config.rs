#[derive(Debug, Clone)]
/// Server configuration
/// - `bind_addr`: The address and port to which api requests can be directed
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
/// - `chain_key`: Chain key for the source chain, must match the chain key on creditcoin3
/// - `eth_rpc_url`: Ethereum RPC url
/// - `enable_prometheus_metrics`:
/// - `prometheus_host`:
/// - `prometheus_port`:
pub struct Config {
    pub bind_addr: String,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub chain_key: u64,
    pub eth_rpc_url: String,
    pub enable_prometheus_metrics: bool,
    pub prometheus_host: String,
    pub prometheus_port: u16,
}
