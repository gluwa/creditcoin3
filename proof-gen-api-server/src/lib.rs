use anyhow::Result;
use anyhow::{anyhow, Result};
use attestor_primitives::{block::ContinuityProof, ContinuityBlock};
use cc_client::Client as CcClient;
use config::Config;
use config::Config;
use db::DbManager;
use db::{DbManager, QueryProofs};
use eth::Client as EthClient;
use mmr::query_proof::{MerkleProofEntry, QueryMerkleProof};
use networking::{build_app, run_http_server};
use sp_core::H256;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::signal;
use tokio::{
    select,
    sync::mpsc::{unbounded_channel, UnboundedSender},
    sync::oneshot::channel,
    time::{sleep, Duration},
};
use tracing::debug;
use tracing::{error, info};

pub mod config;
pub mod db;
mod networking;
mod prom;
mod services;

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;
pub use services::mock_providers;

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
    // TODO: Use this to get blocks and construct proofs!
    #[allow(unused)]
    source_chain_client: EthClient,
    // Prometheus metrics server, if enabled
    // TODO: Actually increment metrics where appropriate
    #[allow(unused)]
    metrics: Option<ProofGenServerMetrics>,
}

impl Server {
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
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

        pub async fn run(&mut self) -> Result<()> {
            self.db_manager.run_migrations().await?;
            let addr: SocketAddr = self
                .config
                .bind_addr
                .parse()
                .expect("Invalid BIND_ADDR format");

            let continuity_config = continuity::ContinuityConfig {
                cc3_rpc_url: self.config.cc3_rpc_url.clone(),
                eth_rpc_url: self.config.eth_rpc_url.clone(),
                chain_key: self.config.chain_key,
            };
            let use_mocks = std::env::var("USE_MOCK_PROVIDERS")
                .ok()
                .map(|v| v == "1")
                .unwrap_or(false);
            let builder = if use_mocks {
                let (cc_mock, eth_mock) =
                    services::mock_providers::make_mock_providers(self.config.chain_key);
                continuity::ContinuityBuilder::new_with_providers(
                    continuity_config,
                    cc_mock,
                    eth_mock,
                )
            } else {
                continuity::ContinuityBuilder::new(continuity_config).await?
            };
            let service = Arc::new(ContinuityService::new(
                Arc::new(builder),
                Arc::new(self.db_manager.clone()),
            ));

            // Define server future
            let app = build_app(service);
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
            let continuity_proof = ContinuityProof {
                lower_endpoint_digest: H256::random(),
                blocks: vec![ContinuityBlock {
                    root: H256::random(),
                    digest: H256::random(),
                }],
            };
            let merkle_proof = QueryMerkleProof {
                root: H256::random(),
                siblings: vec![MerkleProofEntry {
                    hash: H256::random(),
                    is_left: false,
                }],
            };
            let mock_full_block = QueryProofs {
                chain_key: 1,
                header_number: 1,
                tx_index: None,
                tx_hash: None,
                continuity_proof: Some(continuity_proof.clone()),
                merkle_proof: Some(merkle_proof.clone()),
                merkle_root: Some(H256::zero()),
            };
            let mock_tx_proofs = QueryProofs {
                chain_key: 1,
                header_number: 1,
                tx_index: Some(1),
                tx_hash: Some(H256::zero()),
                continuity_proof: Some(continuity_proof),
                merkle_proof: Some(merkle_proof),
                merkle_root: Some(H256::zero()),
            };

            loop {
                // Test insert
                self.db_manager.insert_proofs_entry(mock_full_block.clone());
                self.db_manager.insert_proofs_entry(mock_tx_proofs.clone());
                // Test read
                info!("Waiting on insert before read...");
                sleep(Duration::from_secs(1)).await;
                let maybe_entry = self.db_manager.get_proofs_for_block(1, 1).await?;
                let maybe_id = maybe_entry.map(|e| e.id);
                info!("Full block entry id: {maybe_id:?}");
                let maybe_entry = self
                    .db_manager
                    .get_proofs_by_tx_hash(1, H256::zero())
                    .await?;
                let maybe_id = maybe_entry.map(|e| e.id);
                info!("By hash entry id: {maybe_id:?}");
                let maybe_entry = self.db_manager.get_proofs_for_tx(1, 1, 1).await?;
                info!("Tx full entry: {maybe_entry:?}");
                // Wait a bit to avoid spam
                info!("Waiting...");
                sleep(Duration::from_secs(20)).await;
                let _ = attestation_events_sender.send(AttestationEvent::NewAttestation);
            }
        }
    }

    pub async fn shutdown_signal() {
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
}
