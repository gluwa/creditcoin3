use anyhow::Result;
use eth::Client;
use tokio::sync::{mpsc, oneshot};
use tokio::{fs::File, io::AsyncReadExt};
use tracing::{debug, error, info, warn};

pub mod attestation_cache;
pub mod cc3;
pub mod claim;
pub mod config;
pub mod postgres;

use cc3::Claim;
use config::Config;

#[derive(Debug)]
/// Prover server is configured using `Config`
pub struct Server {
    #[allow(dead_code)]
    config: Config,
    // Channel to send cancellation to the claim subscription
    // will exit when this is dropped
    cancel_tx: Option<oneshot::Sender<()>>,
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server {
            config,
            cancel_tx: None,
        }
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        let cc3_client = cc3::Client::new(
            &self.config.cc3_rpc_url,
            &self.config.cc3_key,
            &self.config.nickname,
        )?;
        debug!("Creating cc3 client");
        cc3_client.init().await?;

        // Sync chain prices configuration
        cc3_client
            .sync_chain_prices_configuration(self.config.chain_price_configurations.chain.clone())
            .await?;

        // Handle claim subscription
        // This will run in the background
        // The server will continue to run and do other work
        self.handle_claim_sub(&cc3_client)?;

        Ok(())
    }

    pub fn handle_claim_sub(&mut self, cc3_client: &cc3::Client) -> Result<()> {
        let (claim_tx, mut claim_rx) = mpsc::channel(self.config.claim_buffer.into());
        debug!(
            "Created claim buffer with size: {}",
            self.config.claim_buffer
        );

        debug!("Starting claim sub on cc3");
        // Create a channel to cancel the claim subscription
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        // Store the cancel_tx so we can cancel the subscription later
        self.cancel_tx = Some(cancel_tx);
        // Run sub in background and allow server to continue doing other work
        let client = cc3_client.clone();
        tokio::spawn(async move {
            let _ = client.start_claim_sub(cancel_rx, claim_tx).await;
        });

        debug!("Starting claim processing handler");
        let chain_price_configurations = self.config.chain_price_configurations.clone();
        // Handle claims in the main task or another spawned task
        let client = cc3_client.clone();
        tokio::spawn(async move {
            while let Some(claim) = claim_rx.recv().await {
                // Get the rpc url for the chain the claim is from
                let eth_client_rpc_url = chain_price_configurations
                    .get_rpc_url(claim.claim.chain_id)
                    .ok_or(anyhow::anyhow!("Chain not found"))
                    .unwrap_or_else(|_| panic!("Chain with id {} not found", claim.claim.chain_id));

                // Create an eth client
                let eth_client = eth::Client::new(eth_client_rpc_url)
                    .await
                    .expect("Error creating eth client");

                // Process the claim
                match process_claim(client.clone(), eth_client, claim).await {
                    Ok(()) => {
                        info!("Claim processed");
                    }
                    Err(e) => {
                        panic!("Error processing claim: {e}, unwinding server..")
                    }
                }
            }
        });

        Ok(())
    }
}

pub async fn process_claim(
    client: crate::cc3::Client,
    eth_client: Client,
    claim: Claim,
) -> Result<()> {
    info!("Processing claim with hash: {:?}", claim.hash);

    // Check if claim exists on source chain
    match claim::check_claim_inclusion(eth_client, claim.claim).await {
        Ok(true) => {
            info!("Claim included on source chain");
        }
        Ok(false) => {
            warn!("Claim not included on source chain");
        }
        Err(e) => {
            error!("Error checking claim inclusion: {:?}", e);
        }
    };

    // Create proof (TODO: hook up prover)
    let mut proof_example = File::open("proof_example.json").await?;

    // Create a buffer to read the file
    let mut proof = Vec::new();

    // read the whole proof file into the buffer
    proof_example.read_to_end(&mut proof).await?;

    // Submit result to cc3
    client.submit_proof(claim.hash, proof).await?;

    Ok(())
}
