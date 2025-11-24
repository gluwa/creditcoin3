use anyhow::{Context, Result};
use std::env;

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

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        // Strings with defaults
        let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
        let cc3_rpc_url =
            env::var("CC3_RPC_URL").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string());
        let eth_rpc_url =
            env::var("ETH_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8545".to_string());

        // Creditcoin account key (mnemonic or seed); require presence
        let cc3_key = env::var("CC3_KEY")
            .context("Missing CC3_KEY environment variable (Creditcoin mnemonic / seed)")?;

        // Optional numeric values
        let chain_key = env::var("CHAIN_KEY")
            .unwrap_or_else(|_| "1".to_string())
            .parse::<u64>()
            .context("Invalid CHAIN_KEY: expected integer")?;

        // Boolean env vars should handle many truthy values
        let enable_prometheus_metrics = env::var("ENABLE_PROMETHEUS_METRICS")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .context("Invalid ENABLE_PROMETHEUS_METRICS: expected true/false")?;

        // Host/Port for metrics
        let prometheus_host = env::var("PROMETHEUS_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let prometheus_port = env::var("PROMETHEUS_PORT")
            .unwrap_or_else(|_| "9090".to_string())
            .parse::<u16>()
            .context("Invalid PROMETHEUS_PORT: expected u16")?;

        Ok(Self {
            bind_addr,
            cc3_rpc_url,
            cc3_key,
            chain_key,
            eth_rpc_url,
            enable_prometheus_metrics,
            prometheus_host,
            prometheus_port,
        })
    }
}
