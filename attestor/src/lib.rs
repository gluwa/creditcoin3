use anyhow::Result;
use eth::Client;
use kameo::spawn;
use tracing::debug;

pub mod attestation;
pub mod cc3;
pub mod eth_sub;
pub mod merkle;
use cc_client::Client as CcClient;

const CHAIN_ID_TO_CHAIN_NAME: [(u64, &'static [u8]); 3] = [
    (1, "Ethereum".as_bytes()),
    (31337, "Local anvil".as_bytes()),
    (11155111, "Sepolia ethereum".as_bytes()),
];

#[derive(Debug, Clone)]
/// Attestor server is configured using `Config`
pub struct Server {
    config: Config,
}

#[derive(Debug, Clone)]
/// Server configuration
/// - `eth_rpc_url`: Source chain RPC url
/// - `eth_start_block`: Start block for the source chain
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
pub struct Config {
    pub eth_rpc_url: String,
    pub eth_start_block: Option<u64>,
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    //pub bls_key: [u8; 32],
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&self) -> Result<()> {
        let eth_client = Client::new(&self.config.eth_rpc_url).await?;

        let chain_id = eth_client.get_chain_id().await?;
        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let chain_name = CHAIN_ID_TO_CHAIN_NAME
            .iter()
            .find(|(id, _)| *id == chain_id)
            .expect("Unknown chain id");

        debug!("Chain name: {:?}", chain_name);

        let chain_key =
            CcClient::get_chain_key(&self.config.cc3_rpc_url, chain_id, chain_name.1.to_vec())
                .await?
                .expect(
                    format!(
                        "Failed to get chain key for chain id {:?} and chain name {:?}",
                        chain_id, chain_name.1
                    )
                    .as_str(),
                );

        debug!("Chain key: {:?}", chain_key);

        let cc3_client = cc3::Client::new(
            &self.config.cc3_rpc_url,
            &self.config.cc3_key,
            chain_key,
            //&self.config.bls_key,
        )
        .await?;
        cc3_client.init().await?;

        let attestation_interval = cc3_client.attestation_interval;

        // Create an Actor reference for the cc3 client
        let cc3_client = spawn(cc3_client);

        // Create an attestor
        let attestor = spawn(attestation::Attestor::new());

        // Subscribe to new eth head given the attestor and cc3 client
        eth_sub::subscribe_to_new_heads(
            eth_client,
            attestor,
            cc3_client,
            self.config.eth_start_block,
            attestation_interval,
            chain_key,
        )
        .await?;

        Ok(())
    }
}
