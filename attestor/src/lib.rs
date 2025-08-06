use anyhow::Result;
use engine::AsyncEngine;
use tokio::{sync::mpsc, time::sleep};
use tracing::{debug, error, info, warn};

mod attestation;
mod cc3;
mod ccsub;
mod continuity;
pub mod engine;
mod error;
mod eth_sub;
mod prom;
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
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub maturity_delay: u64,
    pub chain_key: u64,
    pub enable_prometheus_metrics: bool,
    pub prometheus_host: String,
    pub prometheus_port: u16,
    //pub bls_key: [u8; 32],
}

impl Server {
    /// Create a new server based on `Config`
    #[must_use]
    pub fn new(config: Config) -> Self {
        Server { config }
    }

    pub async fn run(&mut self) -> Result<()> {
        let (shutdown_send, mut shutdown_recv) = mpsc::unbounded_channel::<()>();

        let mut engine = AsyncEngine::new(&self.config, shutdown_send).await?;
        engine.start().await?;
        debug!("Started attestation engine");

        // Create a task for ccsub and monitor it
        let mut ccsub_engine = engine.clone();
        let mut ccsub_handle = tokio::spawn(async move { ccsub::run(&mut ccsub_engine).await });

        loop {
            tokio::select! {
                _ = shutdown_recv.recv() => {
                    engine.stop().await;
                    panic!("Attestor server stopped by shutdown signal");
                }
                ccsub_result = &mut ccsub_handle => {
                    match ccsub_result {
                        Ok(Ok(())) => {
                            panic!("CclientSub completed successfully, this should not happen in a long-running attestor");
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
                                    warn!("Failed to attest, attestor not selected.");
                                } else if e.is_not_running_error() {
                                    info!("Engine not running, continuing ...");
                                } else if e.is_double_vote_error() {
                                    debug!("Double vote detected, continuing ...");
                                } else if e.is_fragment_error() {
                                    warn!("Fragment error detected, exiting ...");
                                    return Err(e.into());
                                } else if e.is_attested_to_error() {
                                    debug!("Attestation already submitted for round {:?}, skipping", round);
                                } else {
                                    error!("Failed to submit attestation for round {:?}: {:?}", round, e);
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
    }
}
