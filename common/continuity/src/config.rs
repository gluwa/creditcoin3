//! Continuity Configuration

/// Configuration for continuity proof generation.
#[derive(Debug, Clone)]
pub struct ContinuityConfig {
    /// CC3 RPC endpoint
    pub cc3_rpc_url: String,
    /// Ethereum RPC endpoint
    pub eth_rpc_url: String,
    /// Chain key for attestation lookup
    pub chain_key: u64,
}

impl ContinuityConfig {
    pub fn new(
        cc3_rpc_url: impl Into<String>,
        eth_rpc_url: impl Into<String>,
        chain_key: u64,
    ) -> Self {
        Self {
            cc3_rpc_url: cc3_rpc_url.into(),
            eth_rpc_url: eth_rpc_url.into(),
            chain_key,
        }
    }
}
