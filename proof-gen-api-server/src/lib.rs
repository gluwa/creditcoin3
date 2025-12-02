use anyhow::{anyhow, bail, Context, Result};
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::{select, sync::oneshot::channel};
use tracing::{error, info};

use crate::prom::ProofGenServerMetrics;
use cc_client::Client as CcClient;
use config::Config;
use db::DbManager;
use eth::Client as EthClient;
use networking::run_http_server;

pub mod config;
pub mod db;
pub mod networking;
mod prom;
pub mod services;

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;

pub struct Server {
    // proof-gen-api-server is configured using `Config`
    config: Config,
    // The db manager, which owns a connection thread pool
    db_manager: DbManager,
    // Client which allows us to request info from Creditcoin3 and follow events
    // TODO: Use this to follow attestation events!
    #[allow(unused)]
    cc3_client: Option<CcClient>,
    // Client which lets us retrieve source chain blocks
    // TODO: Use this to get blocks and construct proofs!
    #[allow(unused)]
    source_chain_client: Option<EthClient>,
    // Prometheus metrics server, if enabled
    // TODO: Actually increment metrics where appropriate
    #[allow(unused)]
    metrics: Option<ProofGenServerMetrics>,
}

impl Server {
    /// Create a new server based on `Config`.
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
        // Initialize CC3 client (read-only, no keypair needed)
        let cc3_client = CcClient::new_read_only(&config.cc3_rpc_url).await?;

        // Validate supported chain and source chain id alignment
        let supported_chain = cc3_client
            .get_supported_chain(config.chain_key)
            .await?
            .ok_or(anyhow!("Failed to get chain key"))?;
        let supported_chain_id = supported_chain.chain_id;

        // Initialize source chain client and validate chain id matches
        let source_chain_client = EthClient::new(&config.eth_rpc_url, None).await?;
        let chain_id = source_chain_client.chain_id();
        if supported_chain_id != chain_id {
            bail!("Wrong chain. Source chain endpoint chain id: {chain_id}, Supported chain id: {supported_chain_id}");
        }

        // Register metrics server if configured
        let metrics = if config.enable_prometheus_metrics {
            info!(
                "📈 Starting Prometheus metrics server on http://{}:{}/metrics",
                config.prometheus_host, config.prometheus_port
            );
            prom::start_prom_server(&config)
        } else {
            None
        };
        {
            let chain_name = cc3_client
                .get_chain_name()
                .await
                .context("cc3_client failed to get chain_name")?;
            info!(
                "✅ Connected to Creditcoin chain: {} with id: {}",
                chain_name, config.chain_key
            );
        }

        Ok(Server {
            config,
            db_manager,
            cc3_client: Some(cc3_client),
            source_chain_client: Some(source_chain_client),
            metrics,
        })
    }

    pub async fn run(&self) -> Result<()> {
        // Run migrations (only after passing guard)
        self.db_manager.run_migrations().await?;

        // Continuity builder configuration
        // Note: cc3_key is not needed for read-only operations, but ContinuityConfig still requires it
        // We use a dummy key since it won't be used for signing
        let continuity_config = continuity::ContinuityConfig {
            cc3_rpc_url: self.config.cc3_rpc_url.clone(),
            cc3_key: self
                .config
                .cc3_key
                .clone()
                .unwrap_or_else(|| "//Alice".to_string()),
            eth_rpc_url: self.config.eth_rpc_url.clone(),
            chain_key: self.config.chain_key,
        };
        // Always use real providers for continuity
        let builder = continuity::ContinuityBuilder::new(continuity_config).await?;

        let service = Arc::new(services::continuity_service::ContinuityService::new(
            Arc::new(builder),
            Arc::new(self.db_manager.clone()),
        ));

        // Build axum application
        let app = build_app(service, self.config.chain_key);
        let (http_shutdown_tx, http_shutdown_rx) = channel::<()>();
        let server = run_http_server(app, &self.config.bind_addr, http_shutdown_rx);
        tokio::pin!(server);

        info!("Server listening on {}", self.config.bind_addr);

        // Define attestation event listening channel – placeholder for future attestation stream
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
                    bail!("API HTTP server exited!");
                }
                // Run DB Tests – temporary hook for ongoing DB design iterations
                res = self.run_db_tests(attestation_events_tx.clone()) => {
                    bail!("Db tests completed: {res:?}")
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
}

// TODO: Implement this event types enum based on all the cc3 attestation events we want to listen for.
// Probably want to implement in a different file.
#[derive(Debug)]
pub enum AttestationEvent {
    NewAttestation,
}

// Temporary DB tests hook. Replace with CI/Integration tests when DB design stabilizes.
impl Server {
    async fn run_db_tests(&self, _tx: UnboundedSender<AttestationEvent>) -> Result<()> {
        // Placeholder: pending future that never completes, keeping the select branch inactive.
        // Replace with actual DB tests implementation when ready.
        std::future::pending().await
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

    info!("Shutdown signal received");
}
