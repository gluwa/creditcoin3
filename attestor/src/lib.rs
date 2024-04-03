use anyhow::Result;
use cc3::Client;
use kameo::{ActorRef, Spawn};

pub mod attestation;
pub mod cc3;
pub mod eth;

#[derive(Debug, Clone)]
pub struct Server<'a> {
    config: Config<'a>,
}

#[derive(Debug, Clone)]
pub struct Config<'a> {
    pub eth_rpc_url: &'a str,
    pub cc3_rpc_url: &'a str,
    pub cc3_key: &'a str,
}

impl<'a> Server<'a> {
    pub fn new(config: Config<'a>) -> Self {
        Server { config }
    }

    pub async fn run(&self) -> Result<()> {
        let cc3_client = cc3::Client::new(self.config.cc3_rpc_url, self.config.cc3_key)?;
        let cc3_client_ref: ActorRef<Client> = cc3_client.spawn();

        let attestor = attestation::Attestor::new(cc3_client_ref).spawn();

        eth::subscribe_to_new_heads(self.config.eth_rpc_url, attestor).await?;

        Ok(())
    }
}
