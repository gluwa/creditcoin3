use crate::prom::ProofGenServerMetrics;
use anyhow::{anyhow, Result};
use cc_client::Client as CcClient;
use config::Config;
use db::DbManager;
use eth::Client as EthClient;
use networking::run_http_server;
use std::sync::Arc;
use tokio::signal;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::{select, sync::oneshot::channel};
use tracing::{error, info};

pub mod config;
pub mod db;
pub mod networking;
mod prom;
pub mod services;

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;
// Re-export shared mocks from common continuity crate to avoid duplication
pub use continuity::mock_providers;

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
        // If running with mock providers, skip external client initialization.
        let (cc3_client_opt, source_chain_client_opt) = if config.use_mock_providers {
            (None, None)
        } else {
            // Initialize CC3 client
            let cc3_client = CcClient::new(&config.cc3_rpc_url, &config.cc3_key).await?;

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
                return Err(anyhow!(
                    "Wrong chain. Source chain endpoint chain id: {chain_id}, Supported chain id: {supported_chain_id}"
                ));
            }
            (Some(cc3_client), Some(source_chain_client))
        };

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
        if let Some(ref cc3_client) = cc3_client_opt {
            let chain_name = cc3_client
                .get_chain_name()
                .await
                .unwrap_or_else(|_| "unknown-chain".to_string());
            info!(
                "✅ Connected to Creditcoin chain: {} with id: {}",
                chain_name, config.chain_key
            );
        }

        Ok(Server {
            config,
            db_manager,
            cc3_client: cc3_client_opt,
            source_chain_client: source_chain_client_opt,
            metrics,
        })
    }

    pub async fn run(&self) -> Result<()> {
        // Production guard: Only trigger when RUST_LOG explicitly set to "production" / "prod" (case-insensitive).
        // Avoid substring matches that could falsely trigger (e.g. "reproduction_steps=trace").
        let is_prod_log = std::env::var("RUST_LOG")
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                matches!(v.as_str(), "production" | "prod")
            })
            .unwrap_or(false);
        if self.config.use_mock_providers && is_prod_log {
            return Err(anyhow!(
                "Refusing to start with mock providers in production"
            ));
        }

        // Run migrations (only after passing guard)
        self.db_manager.run_migrations().await?;

        // Continuity builder configuration
        let continuity_config = continuity::ContinuityConfig {
            cc3_rpc_url: self.config.cc3_rpc_url.clone(),
            eth_rpc_url: self.config.eth_rpc_url.clone(),
            chain_key: self.config.chain_key,
        };
        // Use the normalized config flag (accepts 1/true/yes) to decide mock vs real providers.
        let builder = if self.config.use_mock_providers {
            let (cc_provider, eth_provider) =
                services::mock_providers::make_mock_providers(self.config.chain_key);
            continuity::ContinuityBuilder::new_with_providers(
                continuity_config,
                cc_provider,
                eth_provider,
            )
        } else {
            continuity::ContinuityBuilder::new(continuity_config).await?
        };

        let service = Arc::new(services::continuity_service::ContinuityService::new(
            Arc::new(builder),
            Arc::new(self.db_manager.clone()),
        ));

        // Build axum application
        let app = build_app(service);
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
                    return Err(anyhow!("API HTTP server exited!"));
                }
                // Run DB Tests – temporary hook for ongoing DB design iterations
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
        // Placeholder: no-op for now; return Ok to keep select leg active without blocking.
        Ok(())
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
