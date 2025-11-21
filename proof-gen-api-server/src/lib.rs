use anyhow::{anyhow, Result};
use cc_client::Client as CcClient;
use config::Config;
use db::{DbManager, QueryProofs};
use eth::Client as EthClient;
use networking::{build_app, run_http_server};
use sp_core::H256;
use tokio::{
    select,
    sync::mpsc::{unbounded_channel, UnboundedSender},
    sync::oneshot::channel,
    time::{sleep, Duration},
};
use tracing::debug;
use tracing::{error, info};

use crate::prom::ProofGenServerMetrics;

pub mod config;
pub mod db;
mod networking;
mod prom;
mod services;

// TODO: Implement this event types enum based on all the cc3 attestation
// events we want to listen for.
// Probably want to implement in a different file
#[derive(Debug)]
pub enum AttestationEvent {
    NewAttestation,
}

pub struct Server {
    // proof-gen-api-server is configured using `Config`
    config: Config,
    // The db manager, which owns a connection thread pool
    db_manager: DbManager,
    // Client which allows us to request info from Creditcoin3 and follow events
    // TODO: Use this to follow attestation events!
    #[allow(unused)]
    cc3_client: CcClient,
    // Client which lets us retrieve source chain blocks
    #[allow(unused)]
    source_chain_client: EthClient,
    // Prometheus metrics server, if enabled
    // TODO: Actually increment metrics where appropriate
    #[allow(unused)]
    metrics: Option<ProofGenServerMetrics>,
}

impl Server {
    /// Create a new server based on `Config`
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
        // TODO: Use these config fields once the networking side of the server is merged

        let cc3_client = CcClient::new(&config.cc3_rpc_url, &config.cc3_key).await?;

        // Eventually should support multiple chain keys with different source chain rpc endpoints
        let supported_chain = cc3_client
            .get_supported_chain(config.chain_key)
            .await?
            .ok_or(anyhow!("Failed to get chain key"))?;
        let supported_chain_id = supported_chain.chain_id;

        // Check that the source chain id matches the configured chain id
        // This is to prevent misconfiguration
        let source_chain_client = EthClient::new(&config.eth_rpc_url, None).await?;
        let chain_id = source_chain_client.chain_id();
        if supported_chain_id != chain_id {
            return Err(anyhow!("Wrong chain. Source chain endpoint chain id: {chain_id}, Supported chain id: {supported_chain_id}"));
        }

        // Register metrics server if configured
        let metrics = if config.enable_prometheus_metrics {
            let address_str = format!("{}:{}", config.prometheus_host, config.prometheus_port);
            info!(
                "📈 Starting Prometheus metrics server on http://{}/metrics",
                address_str
            );
            prom::start_prom_server(&config)
        } else {
            None
        };

        let chain_name = cc3_client
            .get_chain_name()
            .await
            .unwrap_or_else(|_| "unknown-chain".to_string());
        info!(
            "✅ Connected to Creditcoin chain: {} with id: {}",
            chain_name, config.chain_key
        );

        Ok(Server {
            config,
            db_manager,
            cc3_client,
            source_chain_client,
            metrics,
        })
    }

    /// Runs the server in the background, will start following the configured source chain
    pub async fn run(&mut self) -> Result<()> {
        debug!("Running proof-gen-api-server!");
        self.db_manager.run_migrations().await?;

        // Define server future
        let app = build_app();
        let (http_shutdown_tx, http_shutdown_rx) = channel::<()>();
        let server = run_http_server(app, &self.config.bind_addr, http_shutdown_rx);
        tokio::pin!(server);

        // Define attestation event listening channel
        // Eventually this would be where we listen for new attestations/checkpoints
        let (attestation_events_tx, mut attestation_events_rx) =
            unbounded_channel::<AttestationEvent>();

        // Main server loop
        loop {
            select! {
                // Attestation event received
                event = attestation_events_rx.recv() => {
                    info!("Event: {event:?}");
                }
                // HTTP server completed (only on error or manual shutdown)
                res = &mut server => {
                    if let Err(err) = res {
                        error!("❌ HTTP server exited with error: {err}");
                    }
                    return Err(anyhow!("API HTTP server exited!"));
                }
                // Run DB Tests
                res = self.run_db_tests(attestation_events_tx.clone()) => {
                    return Err(anyhow!("Db tests completed: {res:?}"))
                }
                // Global shutdown (Ctrl+C / SIGTERM)
                _ = shutdown_signal() => {
                    // Shut down axum http layer
                    let _ = http_shutdown_tx.send(());
                    tracing::info!("🛑 Global shutdown requested, exiting main loop");
                    return Ok(())
                }
            }
        }
    }

    async fn run_db_tests(
        &self,
        attestation_events_sender: UnboundedSender<AttestationEvent>,
    ) -> Result<()> {
        let mock_entry = QueryProofs {
            chain_key: 1,
            header_number: 1,
            tx_index: None,
            tx_hash: Some(H256::zero()),
            continuity_proof: None,
            merkle_proof: None,
            merkle_root: Some(H256::zero()),
        };

        loop {
            // Test insert
            self.db_manager.insert_proofs_entry(mock_entry.clone());
            // Test read
            info!("Waiting on insert before read...");
            sleep(Duration::from_secs(1)).await;
            let maybe_entry = self.db_manager.get_proofs_entry(1, 1).await?;
            info!("Entry: {maybe_entry:?}");
            // Wait a bit to avoid spam
            info!("Waiting...");
            sleep(Duration::from_secs(20)).await;
            let _ = attestation_events_sender.send(AttestationEvent::NewAttestation);
        }
    }
}

pub async fn shutdown_signal() {
    use tokio::signal;

    // Ctrl+C
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        sigterm.recv().await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    tracing::info!("Shutdown signal received");
}
