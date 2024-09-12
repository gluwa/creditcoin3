use anyhow::Result;
use eth::Client;
use kameo::spawn;
use tokio::sync::mpsc;
use tracing::{debug, info};

pub mod attestation;
pub mod cc3;
pub mod eth_sub;
pub mod merkle;

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
        let eth_client = Client::new(&self.config.eth_rpc_url, &String::new()).await?;

        let chain_id = eth_client.get_chain_id().await?;
        debug!("Opened connection to ethereum chain with id {}", chain_id);

        let cc3_client =
            cc3::Client::new(&self.config.cc3_rpc_url, &self.config.cc3_key, chain_id).await?;

        cc3_client.init().await?;

        let attestation_interval = cc3_client.get_attestation_interval();
        debug!("----- Attestation interval: {:?}", attestation_interval);

        let chain_key = cc3_client.get_chain_key();
        debug!("----- Chain key: {:?}", chain_key);

        let (sender, mut receiver) = mpsc::unbounded_channel();
        let cc3_client_for_t1 = cc3_client.clone();
        tokio::spawn(async move {
            cc3_client_for_t1
                .subscribe_randomness_change(sender, chain_id)
                .await
        });

        let cc_client_clone = cc3_client.clone();
        loop {
            // We use this as a trigger to restart the attestation process
            // Eligibility for attestation is determined ever epoch change
            // Randomness is refreshed every epoch change, this is the trigger
            while let Some(randomness) = receiver.recv().await {
                let last_attestation = cc_client_clone
                    .get_last_attestation(cc_client_clone.get_chain_key())
                    .await?;

                let target_header = if let Some(last_attestation) = last_attestation {
                    info!("Last attestation: {:?}", last_attestation);
                    last_attestation.header_number() + attestation_interval
                } else {
                    info!("No last attestation found, starting from 0");
                    0
                };

                match cc_client_clone.sign_vrf_for_header(target_header).await {
                    Ok(_) => {
                        info!(
                            "Attestor selected for attesting in this epoch slot: {}",
                            randomness.0
                        );
                        // Create an Actor reference for the cc3 client
                        let cc3_client = spawn(cc_client_clone.clone());
                        let eth_client = eth_client.clone();

                        // Create an attestor
                        let attestor = spawn(attestation::Attestor::default());

                        info!(
                            "Subscribing to new heads at target block: {}",
                            target_header
                        );
                        // Subscribe to new eth head given the attestor and cc3 client
                        eth_sub::subscribe_to_new_heads(
                            eth_client,
                            attestor,
                            cc3_client,
                            target_header,
                            attestation_interval,
                        )
                        .await?;
                    }
                    Err(e) => {
                        info!("Attestor not selected in this epoch slot.");
                        debug!("Error: {:?}", e);
                    }
                }
            }
        }
    }
}
