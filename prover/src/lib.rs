use anyhow::Result;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info};

pub mod cc3;
pub mod config;
pub mod eth;
pub mod transaction;

use cc3::Claim;
use config::{ChainPriceConfiguration, Config};

#[derive(Debug)]
/// Attestor server is configured using `Config`
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
        let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
        self.cancel_tx = Some(cancel_tx);

        let cc3_client = cc3::Client::new(
            &self.config.cc3_rpc_url,
            &self.config.cc3_key,
            &self.config.nickname,
        )?;
        debug!("Creating cc3 client");
        cc3_client.init().await?;

        // Sync chain prices configuration
        self.sync_chain_prices_configuration(cc3_client.clone())
            .await?;

        let (claim_tx, mut claim_rx) = mpsc::channel(self.config.claim_buffer.into());
        debug!(
            "Created claim buffer with size: {}",
            self.config.claim_buffer
        );

        debug!("Starting claim sub on cc3");
        // Run sub in background and allow server to continue doing other work
        let client = cc3_client.clone();
        tokio::spawn(async move {
            let _ = client.start_claim_sub(cancel_rx, claim_tx).await;
        });

        debug!("Starting claim processing handler");
        // Handle claims in the main task or another spawned task
        let client = cc3_client.clone();
        tokio::spawn(async move {
            while let Some(claim) = claim_rx.recv().await {
                match process_claim(client.clone(), claim).await {
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

    pub async fn sync_chain_prices_configuration(&self, client: crate::cc3::Client) -> Result<()> {
        let chain_price_configurations: Vec<ChainPriceConfiguration> = client
            .cc_client
            .get_chain_price_configurations()
            .await?
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        info!(
            "Syncing chain price configurations: {:?}",
            chain_price_configurations
        );

        // TODO: compare with current configuration and update if needed
        let config_chain_prices: Vec<ChainPriceConfiguration> = self
            .config
            .chain_price_configurations
            .clone()
            .chain
            .into_iter()
            .map(std::convert::Into::into)
            .collect();

        // compare with current configuration and update if needed
        if chain_price_configurations == config_chain_prices {
            info!("Chain price configurations are up to date");
        } else {
            info!("Updating chain price configurations");
            client
                .cc_client
                .update_chain_price_configurations(
                    config_chain_prices
                        .into_iter()
                        .map(std::convert::Into::into)
                        .collect(),
                )
                .await?;
        };

        Ok(())
    }
}

pub async fn process_claim(client: crate::cc3::Client, claim: Claim) -> Result<()> {
    info!("Processing claim: {:?}, client: {:?}", claim, client);

    // Create proof (TODO: hook up prover)
    let proof: Vec<u8> = vec![];

    // Submit result to cc3

    client.submit_proof(claim.hash, proof).await?;

    Ok(())
}
