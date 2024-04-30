use anyhow::Result;
use kameo::spawn;

pub mod attestation;
pub mod cc3;
pub mod eth;
pub mod merkle;
pub mod transaction;

#[derive(Debug, Clone)]
/// Attestor server is configured using `Config`
pub struct Server {
    config: Config,
}

#[derive(Debug, Clone)]
/// Server configuration
/// - `eth_rpc_url`: Source chain RPC url
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
pub struct Config {
    pub eth_rpc_url: String,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub bls_key: [u8; 32],
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&self) -> Result<()> {
        let cc3_client = cc3::Client::new(
            &self.config.cc3_rpc_url,
            &self.config.cc3_key,
            &self.config.bls_key,
        )?;
        cc3_client.init().await?;

        // Create an Actor reference for the cc3 client
        let cc3_client = spawn(cc3_client);

        // Create an attestor
        let attestor = spawn(attestation::Attestor::new());

        // Subscribe to new eth head given the attestor and cc3 client
        eth::subscribe_to_new_heads(&self.config.eth_rpc_url, attestor, cc3_client).await?;

        Ok(())
    }
}
