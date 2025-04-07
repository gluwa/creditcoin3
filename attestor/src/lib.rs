use anyhow::Result;
use std::sync::Arc;
use tokio::{sync::Mutex, time::sleep};
use tracing::{info, warn};

pub mod engine;

mod attestation;
mod cc3;
mod ccsub;
mod eth_sub;
mod fragment;
mod retry;

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
    pub eth_start_block: u64,
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

    pub async fn run(&mut self) -> Result<()> {
        // Construct the attestation engine
        let mut engine = engine::Engine::new(&self.config).await?;
        let chain_key = engine.chain_key();

        // Start the attestation engine
        engine.start(self.config.eth_start_block).await?;
        info!("Started attestation engine");

        // Wrap the engine in a arc mutex
        let engine = Arc::new(Mutex::new(engine));

        let ccsub = ccsub::CclientSub::new(engine.clone(), chain_key);
        ccsub.run().await?;

        // Poll the engine for new attestations
        loop {
            let mut guard = engine.lock().await;

            let maybe_attestation = guard.next().await;

            if let Some(attestation) = maybe_attestation {
                let digest = attestation.digest();
                info!(
                    "Going to submit attestation with digest: {:?}. Round: {:?}",
                    digest,
                    attestation.round()
                );
                match guard.submit_attestation(attestation).await {
                    Ok(()) => {
                        info!("Submitted attestation with digest: {:?}", digest);
                    }
                    Err(e) => {
                        if e.is_not_selected_error() {
                            warn!("Failed to create proof of inclusion, attestor not selected.");
                        } else if e.is_not_running_error() {
                            info!("Engine not running, continuing ...");
                        } else if e.is_double_vote_error() {
                            warn!("Double vote detected, continuing ...");
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            } else {
                drop(guard);
                // sleep
                info!("No attestation to submit, sleeping for 6 seconds");
                sleep(std::time::Duration::from_secs(6)).await;
            }
        }
    }
}
