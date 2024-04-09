use anyhow::Result;
use cc3::Client;
use kameo::{ActorRef, Spawn};

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
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&self) -> Result<()> {
        let cc3_client = cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key)?;
        cc3_client.init().await?;

        // Create an Actor reference for the cc3 client
        let cc3_client_ref: ActorRef<Client> = cc3_client.spawn();

        // Create an attestor
        let attestor = attestation::Attestor::new(cc3_client_ref).spawn();

        // Subscribe to new eth head given the attestor
        eth::subscribe_to_new_heads(&self.config.eth_rpc_url, attestor).await?;

        Ok(())
    }
}
