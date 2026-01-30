use anyhow::{anyhow, bail, Context, Result};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{oneshot::channel, RwLock};
use tokio::{select, signal};
use tracing::{error, info};

use crate::prom::{Metrics, NoopMetrics, ProofGenMetrics};
use cc_client::Client as CcClient;
use config::Config;
use continuity::ContinuityBuilder;
use eth::Client as EthClient;
use networking::run_http_server;

pub mod config;
pub mod events;
pub mod networking;
pub mod prom;
pub mod services;

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;
pub use services::errors::ErrorResponse;

pub struct Server {
    // proof-gen-api-server is configured using `Config`
    config: Config,
    // Client which allows us to request info from Creditcoin3 and follow events
    cc3_client: Arc<CcClient>,
    // Builder for continuity proofs (owns ETH + CC3 clients internally)
    builder: Arc<ContinuityBuilder>,
    // Dynamic checkpoint intervals per chain (updated via events)
    checkpoint_intervals: events::CheckpointIntervalMap,
    // Last checkpoint block number per chain (updated via events)
    // Used to quickly determine if a query needs checkpoint data
    last_checkpoint_blocks: events::LastCheckpointBlockMap,
    // Prometheus metrics, if enabled
    // Store as Arc<ProofGenMetrics> to access block_cache_metrics(), convert to Metrics trait object when needed
    prom_metrics: Option<Arc<ProofGenMetrics>>,
}

impl Server {
    /// Create a new server based on `Config`.
    pub async fn new(config: Config) -> Result<Self> {
        // Create metrics first (if enabled) so we can share the registry with block cache
        let prom_metrics: Option<Arc<ProofGenMetrics>> = if config.enable_prometheus_metrics {
            let prom_host = &config.prometheus_host;
            let prom_port = config.prometheus_port;
            info!("📈 Prometheus metrics enabled on http://{prom_host}:{prom_port}/metrics");
            Some(Arc::new(ProofGenMetrics::new(config.chain_key)))
        } else {
            None
        };

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
            // Get block cache metrics from our metrics (if enabled) or create standalone
            let block_cache_metrics = if let Some(ref m) = prom_metrics {
                m.block_cache_metrics()
            } else {
                // Create standalone metrics (won't be exported, but needed for the interface)
                let mut dummy_registry = prometheus_client::registry::Registry::default();
                eth::metrics::BlockCacheMetrics::new(&mut dummy_registry)
            };
            let cache_config = eth::block_cache::BlockCacheConfig {
                redis_url: redis_url.clone(),
                metrics: block_cache_metrics,
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

        // Fetch intervals from CC3 chain at startup
        let attestation_interval = cc3_client
            .chain_attestation_interval(config.chain_key)
            .await
            .context("Failed to fetch attestation interval")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Attestation interval not configured for chain {}",
                    config.chain_key
                )
            })?;

        let checkpoint_interval = cc3_client
            .chain_checkpoint_interval(config.chain_key)
            .await
            .context("Failed to fetch checkpoint interval")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Checkpoint interval not configured for chain {}",
                    config.chain_key
                )
            })? as u64;

        info!(
            "📊 Intervals for chain {}: {} blocks/attestation, {} attestations/checkpoint ({} blocks/checkpoint)",
            config.chain_key, attestation_interval, checkpoint_interval, attestation_interval * checkpoint_interval
        );

        // Initialize last checkpoint blocks map by fetching the last checkpoint
        let last_checkpoint_blocks = Arc::new(RwLock::new(std::collections::HashMap::new()));
        let last_checkpoint_block = if let Ok(Some(last_checkpoint)) =
            cc3_client.get_last_checkpoint(config.chain_key).await
        {
            let block_number = last_checkpoint.block_number;
            last_checkpoint_blocks
                .write()
                .await
                .insert(config.chain_key, block_number);
            info!(
                "📌 Initialized last checkpoint block for chain {}: {}",
                config.chain_key, block_number
            );
            Some(block_number)
        } else {
            info!(
                "📌 No checkpoint found for chain {} at startup (will be updated via events)",
                config.chain_key
            );
            None
        };

        // Create continuity builder with validated clients
        let continuity_config = continuity::ContinuityConfig::builder()
            .cc3_rpc_url(config.cc3_rpc_url.clone())
            .eth_rpc_url(config.eth_rpc_url.clone())
            .chain_key(config.chain_key)
            .attestation_interval(attestation_interval)
            .checkpoint_interval(checkpoint_interval)
            .last_checkpoint_block(last_checkpoint_block)
            .build();

        // Create indexer client if URL is configured
        let indexer_provider = if let Some(url) = config.indexer_url.as_ref() {
            info!("Indexer provider configured at: {}", url);
            let client = indexer_client::IndexerClient::new(url.clone())?;
            Some(Arc::new(client))
        } else {
            None
        };

        let builder = Arc::new(ContinuityBuilder::new_with_indexer(
            continuity_config,
            cc3_client.clone(),
            eth_client,
            indexer_provider,
        ));

        // Initialize checkpoint intervals map with the fetched value
        let checkpoint_intervals = Arc::new(RwLock::new(std::collections::HashMap::new()));
        checkpoint_intervals
            .write()
            .await
            .insert(config.chain_key, checkpoint_interval);

        Ok(Server {
            config,
            cc3_client,
            builder,
            checkpoint_intervals,
            last_checkpoint_blocks,
            prom_metrics,
        })
    }

    pub async fn run(&self) -> Result<()> {
        // Convert prom_metrics to Metrics trait object, using NoopMetrics as fallback
        // ContinuityService now uses Metrics (non-optional) with NoopMetrics fallback
        let metrics: Metrics = self
            .prom_metrics
            .clone()
            .map(|m| m as Metrics)
            .unwrap_or_else(|| NoopMetrics::new());

        let service = Arc::new(
            services::continuity_service::ContinuityService::new(
                self.builder.clone(),
                metrics.clone(),
            )
            .await?,
        );

        // Build axum application - uses the same Metrics instance
        // Pass prom_metrics for /metrics endpoint on main server
        let app = build_app(
            service,
            self.config.chain_key,
            metrics,
            self.prom_metrics.clone(),
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

        // Start metrics server if configured
        if let Some(ref prom_metrics) = self.prom_metrics {
            let metrics_clone = Arc::clone(prom_metrics);
            let metrics_host = self.config.prometheus_host.clone();
            let metrics_port = self.config.prometheus_port;
            tokio::spawn(async move {
                if let Err(e) = run_metrics_server(metrics_clone, &metrics_host, metrics_port).await
                {
                    error!("Metrics server failed: {e}");
                }
            });
        }

        // Start CC3 event subscription with the configured chain key
        // Events are processed directly in the subscription loop and checkpoint interval changes are monitored
        let cc3_client_clone = self.cc3_client.clone();
        let chain_key = self.config.chain_key;
        let checkpoint_intervals_clone = self.checkpoint_intervals.clone();
        let last_checkpoint_blocks_clone = self.last_checkpoint_blocks.clone();
        let builder_clone = self.builder.clone();

        tokio::spawn(async move {
            if let Err(e) = events::start_cc3_event_subscription(
                cc3_client_clone,
                chain_key,
                checkpoint_intervals_clone,
                last_checkpoint_blocks_clone,
                builder_clone,
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

    /// Get the current checkpoint interval for the configured chain.
    /// This value is dynamically updated when checkpoint interval changes are detected.
    pub async fn get_checkpoint_interval(&self) -> Option<u64> {
        self.checkpoint_intervals
            .read()
            .await
            .get(&self.config.chain_key)
            .copied()
    }

    /// Get the last checkpoint block number for the configured chain.
    /// Returns None if no checkpoint has been reached yet.
    /// This value is dynamically updated when checkpoint events are received.
    pub async fn get_last_checkpoint_block(&self) -> Option<u64> {
        self.last_checkpoint_blocks
            .read()
            .await
            .get(&self.config.chain_key)
            .copied()
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

/// Run the Prometheus metrics HTTP server.
async fn run_metrics_server(metrics: Arc<ProofGenMetrics>, host: &str, port: u16) -> Result<()> {
    use axum::{routing::get, Router};

    async fn handle_metrics(
        axum::extract::State(metrics): axum::extract::State<Arc<ProofGenMetrics>>,
    ) -> impl axum::response::IntoResponse {
        // Update hardware metrics before encoding
        metrics.update_hardware().await;

        axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )
            .body(axum::body::Body::from(metrics.encode()))
            .unwrap()
    }

    let router = Router::new()
        .route("/metrics", get(handle_metrics))
        .with_state(metrics);

    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind metrics server to {addr}"))?;

    info!("📈 Metrics server listening on http://{addr}/metrics");

    axum::serve(listener, router)
        .await
        .context("Metrics server failed")?;

    Ok(())
}
