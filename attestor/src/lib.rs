use anyhow::Result;
use cc3::Client;
use kameo::{ActorRef, Spawn};

pub mod attestation;
pub mod cc3;
pub mod eth;

#[derive(Debug, Clone)]
pub struct Server {
    config: Config,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub eth_rpc_url: String,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&self) -> Result<()> {
        let cc3_client = cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key)?;
        cc3_client.init().await?;

        let cc3_client_ref: ActorRef<Client> = cc3_client.spawn();

        let attestor = attestation::Attestor::new(cc3_client_ref).spawn();

        eth::subscribe_to_new_heads(&self.config.eth_rpc_url, attestor).await?;

        Ok(())
    }
}
