use anyhow::Result;
use engine::AsyncEngine;
use tokio::time::sleep;
use tracing::{debug, info, warn};

pub mod engine;

mod attestation;
mod cc3;
mod ccsub;
mod continuity;
mod error;
mod eth_sub;
mod retry;
mod sync_state;

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
    pub start_block: u64,
    pub maturity_delay: u64,
    //pub bls_key: [u8; 32],
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut engine = AsyncEngine::new(&self.config).await?;
        engine.start(self.config.eth_start_block).await?;
        debug!("Started attestation engine");

        // Create a task for ccsub and monitor it
        let ccsub = ccsub::CclientSub::new(engine.clone());
        let mut ccsub_handle = tokio::spawn(async move { ccsub.run().await });

        loop {
            tokio::select! {
                ccsub_result = &mut ccsub_handle => {
                    match ccsub_result {
                        Ok(Ok(())) => {
                            info!("CclientSub completed successfully");
                            break; // or continue depending on whether it's fatal
                        },
                        Ok(Err(e)) => {
                            return Err(e);
                        },
                        Err(join_err) => {
                            tracing::error!("CclientSub panicked or was aborted: {}", join_err);
                            return Err(join_err.into());
                        }
                    }
                }
                maybe_attestation = engine.next() => {
                    if let Some(attestation) = maybe_attestation {
                        let round = attestation.round();
                        debug!("Going to submit attestation for round: {:?}", round);
                        match engine.submit_attestation(attestation).await {
                            Ok(()) => {
                                debug!("Submitted attestation for round: {:?}", round);
                            }
                            Err(e) => {
                                if e.is_not_selected_error() {
                                    warn!("Failed to create proof of inclusion, attestor not selected.");
                                } else if e.is_not_running_error() {
                                    info!("Engine not running, continuing ...");
                                } else if e.is_double_vote_error() {
                                    warn!("Double vote detected, continuing ...");
                                } else if e.is_fragment_error() {
                                    warn!("Fragment error detected, continuing ...");
                                } else {
                                    return Err(e.into());
                                }
                            }
                        }
                    } else {
                        debug!("No attestation to submit, sleeping for 6 seconds");
                        sleep(std::time::Duration::from_secs(6)).await;
                    }
                }
            }
        }

        Ok(())
    }
}
