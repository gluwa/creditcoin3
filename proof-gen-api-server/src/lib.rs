use anyhow::{anyhow, bail, Context, Result};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::signal;
use tokio::{select, sync::oneshot::channel};
use tracing::{error, info};

use crate::prom::ProofGenServerMetrics;
use cc_client::Client as CcClient;
use config::Config;
use continuity::ContinuityBuilder;
use db::DbManager;
use eth::Client as EthClient;
use networking::run_http_server;

pub mod config;
pub mod db;
pub mod events;
pub mod indexer;
pub mod networking;
mod prom;
pub mod services;

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;
pub use services::errors::ErrorResponse;

pub struct Server {
    // proof-gen-api-server is configured using `Config`
    config: Config,
    // The db manager, which owns a connection thread pool
    db_manager: DbManager,
    // Client which allows us to request info from Creditcoin3 and follow events
    cc3_client: Arc<CcClient>,
    // Builder for continuity proofs (owns ETH + CC3 clients internally)
    builder: Arc<ContinuityBuilder>,
    // Prometheus metrics (always enabled)
    // TODO: Actually increment metrics where appropriate
    #[allow(unused)]
    metrics: ProofGenServerMetrics,
    // Prometheus registry for serving metrics on main API server
    prometheus_registry: Arc<prometheus::Registry>,
}

impl Server {
    /// Create a new server based on `Config`.
    pub async fn new(config: Config, db_manager: DbManager) -> Result<Self> {
        // Initialize CC3 client (read-only, no keypair needed)
        let cc3_client = Arc::<CcClient>::new(CcClient::new_read_only(&config.cc3_rpc_url).await?);

        // Validate supported chain and source chain id alignment
        let supported_chain = cc3_client
            .get_supported_chain(config.chain_key)
            .await?
            .ok_or(anyhow!("Failed to get chain key"))?;
        let supported_chain_id = supported_chain.chain_id;

        // Initialize source chain client and validate chain id matches
        // Use cached client if Redis is configured, otherwise regular client
        let eth_client = if let Some(ref redis_url) = config.redis_url {
            info!("Using Redis block caching at {}", redis_url);
            let cache_config = eth::block_cache::BlockCacheConfig {
                redis_url: redis_url.clone(),
                metrics_registry: Arc::new(prometheus::Registry::new()), // TODO: Add metrics registry from server
            };
            Arc::new(EthClient::new_with_cache(&config.eth_rpc_url, None, cache_config).await?)
        } else {
            Arc::new(EthClient::new(&config.eth_rpc_url, None).await?)
        };
        let chain_id = eth_client.chain_id();
        if supported_chain_id != chain_id {
            bail!("Wrong chain. Source chain endpoint chain id: {chain_id}, Supported chain id: {supported_chain_id}");
        }

        // Log CC3 connection
        let chain_name = cc3_client
            .get_chain_name()
            .await
            .context("cc3_client failed to get chain_name")?;
        info!(
            "✅ Connected to Creditcoin chain: {} with id: {}",
            chain_name, config.chain_key
        );

        // Create continuity builder with validated clients
        let continuity_config = continuity::ContinuityConfig {
            cc3_rpc_url: config.cc3_rpc_url.clone(),
            cc3_key: config
                .cc3_key
                .clone()
                .unwrap_or_else(|| "//Alice".to_string()),
            eth_rpc_url: config.eth_rpc_url.clone(),
            chain_key: config.chain_key,
        };
        let builder = Arc::new(ContinuityBuilder::new_with_providers(
            continuity_config,
            cc3_client.clone(),
            eth_client,
        ));

        // Initialize Prometheus metrics (always enabled, served on main API server)
        info!("📈 Prometheus metrics enabled and available at /metrics");
        let (metrics, prometheus_registry) = prom::init_metrics(&config);

        Ok(Server {
            config,
            db_manager,
            cc3_client,
            builder,
            metrics,
            prometheus_registry,
        })
    }

    pub async fn run(&self) -> Result<()> {
        // Run migrations (only after passing guard)
        self.db_manager.run_migrations().await?;

        // Create Arc<DbManager> once and reuse it (avoid repeated allocations in event loop)
        let db_manager = Arc::new(self.db_manager.clone());

        let service = Arc::new(
            services::continuity_service::ContinuityService::new(
                self.cc3_client.clone(),
                self.builder.clone(),
                Arc::new(self.db_manager.clone()),
            )
            .await?,
        );

        // Build axum application
        let app = build_app(
            service,
            self.config.chain_key,
            self.prometheus_registry.clone(),
        );
        let (http_shutdown_tx, http_shutdown_rx) = channel::<()>();

        // Parse bind address properly to support both IPv4 and IPv6
        let bind_host = &self.config.bind_host;
        let ip = bind_host.parse::<IpAddr>().with_context(|| {
            format!("Invalid bind host: '{bind_host}'. Expected IP address (e.g., '0.0.0.0', '127.0.0.1', '::1', '::')")
        })?;
        let bind_addr = SocketAddr::new(ip, self.config.bind_port);

        let server = run_http_server(app, bind_addr, http_shutdown_rx);
        tokio::pin!(server);

        info!("Server listening on {bind_addr}");

        // Start CC3 event subscription with the configured chain key
        // Events are processed directly in the subscription loop
        let cc3_client_clone = self.cc3_client.clone();
        let db_manager_clone = Arc::clone(&db_manager);
        let chain_key = self.config.chain_key;

        // Create indexer client if URL is configured (for pre-fetching continuity proofs)
        let indexer_client = self.config.indexer_url.as_ref().map(|url| {
            info!("Indexer client configured at: {}", url);
            Arc::new(indexer::IndexerClient::new(url.clone()))
        });

        tokio::spawn(async move {
            if let Err(e) = events::start_cc3_event_subscription(
                cc3_client_clone,
                db_manager_clone,
                chain_key,
                indexer_client,
            )
            .await
            {
                error!("CC3 event subscription failed: {e}");
            }
        });

        // Wait for server exit or shutdown signal
        select! {
            res = &mut server => {
                if let Err(err) = res {
                    error!("❌ HTTP server exited with error: {err}");
                }
                bail!("API HTTP server exited!");
            }
            _ = shutdown_signal() => {
                let _ = http_shutdown_tx.send(());
                tracing::info!("🛑 Global shutdown requested, exiting");
                Ok(())
            }
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

    info!("Shutdown signal received");
}
