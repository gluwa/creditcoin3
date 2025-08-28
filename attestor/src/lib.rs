use anyhow::Result;
use engine::AttestorService;
use tokio::time::sleep;
use tracing::{debug, info, warn};

mod cc3;
mod continuity;
mod engine;
mod error;
mod prom;
mod util;

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
        // Spin up the service actor
        let handle = AttestorService::spawn(&self.config).await?;
        debug!("Started attestation service");

        // Example: consumer of published attestations (optional)
        let mut rx = handle.subscribe_attestations();

        loop {
            tokio::select! {
                // Graceful shutdown on Ctrl-C
                _ = tokio::signal::ctrl_c() => {
                    info!("Received Ctrl-C, shutting down attestation service…");
                    // Ask the service to stop and break out
                    let _ = handle.shutdown().await;
                    break;
                }
                msg = rx.recv() => {
                    match msg {
                        Ok(_att) => {
                            // You can log or forward attestations here if desired
                            debug!("Received attestation");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            warn!("Attestation stream closed; exiting");
                            break;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                    }
                }
                // In a simple server, idle a bit
                () = sleep(std::time::Duration::from_secs(6)) => {}
            }
        }
        Ok(())
    }
}
