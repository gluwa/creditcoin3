use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{oneshot::channel, RwLock};
use tokio::{select, signal};
use tracing::{debug, error, info};

use crate::prom::{Metrics, ProofGenMetrics};
use cc_client::Client as CcClient;
use continuity::ContinuityBuilder;
use eth::Client as EthClient;
use networking::run_http_server;

pub mod config;
pub mod events;
pub mod networking;
pub mod prom;
pub mod services;

pub use config::{ChainConfig, Config, DEFAULT_MAX_BATCH_SIZE};

// Re-exports for integration tests and external callers
pub use networking::build_app;
pub use services::continuity_service::ContinuityService;
pub use services::errors::ErrorResponse;

/// Hide `?query` in logs so keys in URLs are not fully printed.
fn redact_url_query(url: &str) -> String {
    url.split_once('?')
        .map(|(base, _)| format!("{base}?…"))
        .unwrap_or_else(|| url.to_string())
}

pub struct Server {
    config: Config,
    cc3_client: Arc<CcClient>,
    /// One continuity builder per configured source chain.
    builders: Vec<Arc<ContinuityBuilder>>,
    checkpoint_intervals: events::CheckpointIntervalMap,
    last_checkpoint_blocks: events::LastCheckpointBlockMap,
    prom_metrics: Arc<ProofGenMetrics>,
}

impl Server {
    /// Create a new server based on `Config`.
    pub async fn new(config: Config) -> Result<Self> {
        let chain_keys: Vec<u64> = config.chains.iter().map(|c| c.chain_key).collect();
        let prom_metrics = Arc::new(ProofGenMetrics::new(&chain_keys));
        info!("📈 Prometheus metrics available at /metrics");

        debug!(
            cc3_rpc_url = %config.cc3_rpc_url,
            chain_count = config.chains.len(),
            "[startup] connecting Creditcoin3 read-only client (cc3_rpc_url)"
        );
        let cc3_client = Arc::<CcClient>::new(
            CcClient::new_read_only(&config.cc3_rpc_url)
                .await
                .with_context(|| {
                    format!(
                        "Creditcoin3 RPC failed at cc3_rpc_url={}. \
                         Ensure the node is up, the URL scheme (ws/wss) matches, and network/firewall allows the connection.",
                        config.cc3_rpc_url
                    )
                })?,
        );
        debug!("[startup] Creditcoin3 client connected");

        let mut builders: Vec<Arc<ContinuityBuilder>> = Vec::with_capacity(config.chains.len());
        let checkpoint_intervals = Arc::new(RwLock::new(HashMap::new()));
        let last_checkpoint_blocks = Arc::new(RwLock::new(HashMap::new()));

        for (idx, chain) in config.chains.iter().enumerate() {
            debug!(
                step = idx + 1,
                of = config.chains.len(),
                chain_key = chain.chain_key,
                eth_rpc_url = %redact_url_query(&chain.eth_rpc_url),
                archiver_url = ?chain.archiver_url.as_ref().map(|u| redact_url_query(u)),
                "[startup] configuring source chain"
            );
            let builder = Self::build_continuity_for_chain(
                &config,
                cc3_client.clone(),
                chain,
                prom_metrics.clone(),
                &checkpoint_intervals,
                &last_checkpoint_blocks,
            )
            .await?;
            builders.push(builder);
        }

        Ok(Server {
            config,
            cc3_client,
            builders,
            checkpoint_intervals,
            last_checkpoint_blocks,
            prom_metrics,
        })
    }

    async fn build_continuity_for_chain(
        global: &Config,
        cc3_client: Arc<CcClient>,
        chain: &ChainConfig,
        prom_metrics: Arc<ProofGenMetrics>,
        checkpoint_intervals: &Arc<RwLock<HashMap<u64, u64>>>,
        last_checkpoint_blocks: &Arc<RwLock<HashMap<u64, u64>>>,
    ) -> Result<Arc<ContinuityBuilder>> {
        let chain_key = chain.chain_key;

        debug!(
            chain_key,
            "[startup] querying CC3 for supported chain metadata (get_supported_chain)"
        );
        let supported_chain = cc3_client
            .get_supported_chain(chain_key)
            .await
            .with_context(|| {
                format!("CC3 RPC call get_supported_chain failed for chain_key={chain_key}")
            })?
            .ok_or_else(|| anyhow!("Failed to get supported chain for chain_key {chain_key}"))?;
        let supported_chain_id = supported_chain.chain_id;

        let eth_client = if let Some(ref redis_url) = global.redis_url {
            debug!(
                chain_key,
                redis_url = %redact_url_query(redis_url),
                cluster_mode = global.redis_cluster_mode,
                eth_rpc_url = %redact_url_query(&chain.eth_rpc_url),
                "[startup] connecting source chain ETH client with Redis block cache"
            );
            let block_cache_metrics = prom_metrics.block_cache_metrics();
            let cache_config = eth::block_cache::BlockCacheConfig {
                redis_url: redis_url.clone(),
                redis_cluster_mode: global.redis_cluster_mode,
                metrics: block_cache_metrics,
            };
            Arc::new(
                EthClient::new_with_cache(&chain.eth_rpc_url, None, cache_config)
                    .await
                    .with_context(|| {
                        format!(
                            "Ethereum/source RPC + Redis cache failed for chain_key={chain_key} (eth_rpc_url={}, redis={})",
                            redact_url_query(&chain.eth_rpc_url),
                            redact_url_query(redis_url)
                        )
                    })?,
            )
        } else {
            debug!(
                chain_key,
                eth_rpc_url = %redact_url_query(&chain.eth_rpc_url),
                "[startup] connecting source chain ETH client (no Redis)"
            );
            Arc::new(
                EthClient::new(&chain.eth_rpc_url, None)
                    .await
                    .with_context(|| {
                        format!(
                            "Ethereum/source RPC connection failed for chain_key={chain_key} (eth_rpc_url={})",
                            redact_url_query(&chain.eth_rpc_url)
                        )
                    })?,
            )
        };

        let chain_id = eth_client.chain_id();
        if supported_chain_id != chain_id {
            bail!(
                "Wrong chain for chain_key {chain_key}. Source chain endpoint chain id: {chain_id}, Supported chain id: {supported_chain_id}"
            );
        }

        let attestation_interval = cc3_client
            .chain_attestation_interval(chain_key)
            .await
            .context("Failed to fetch attestation interval")?
            .ok_or_else(|| {
                anyhow::anyhow!("Attestation interval not configured for chain {chain_key}")
            })?;

        let checkpoint_interval = cc3_client
            .chain_checkpoint_interval(chain_key)
            .await
            .context("Failed to fetch checkpoint interval")?
            .ok_or_else(|| {
                anyhow::anyhow!("Checkpoint interval not configured for chain {chain_key}")
            })? as u64;

        debug!(
            "📊 Intervals for chain {}: {} blocks/attestation, {} attestations/checkpoint ({} blocks/checkpoint)",
            chain_key,
            attestation_interval,
            checkpoint_interval,
            attestation_interval * checkpoint_interval
        );

        let last_checkpoint_block =
            if let Ok(Some(last_checkpoint)) = cc3_client.get_last_checkpoint(chain_key).await {
                let block_number = last_checkpoint.block_number;
                last_checkpoint_blocks
                    .write()
                    .await
                    .insert(chain_key, block_number);
                debug!(
                    "📌 Initialized last checkpoint block for chain {}: {}",
                    chain_key, block_number
                );
                Some(block_number)
            } else {
                debug!(
                    "📌 No checkpoint found for chain {} at startup (will be updated via events)",
                    chain_key
                );
                None
            };

        let continuity_config = continuity::ContinuityConfig::builder()
            .cc3_rpc_url(global.cc3_rpc_url.clone())
            .eth_rpc_url(chain.eth_rpc_url.clone())
            .chain_key(chain_key)
            .attestation_interval(attestation_interval)
            .checkpoint_interval(checkpoint_interval)
            .last_checkpoint_block(last_checkpoint_block)
            .build();

        let indexer_provider = if let Some(url) = global.indexer_url.as_ref() {
            debug!(
                chain_key,
                indexer_url = %redact_url_query(url),
                "[startup] building indexer GraphQL client"
            );
            let client = indexer_client::IndexerClient::new(url.clone()).with_context(|| {
                format!(
                    "Indexer client init failed for chain_key={chain_key} (indexer_url={})",
                    redact_url_query(url)
                )
            })?;
            Some(Arc::new(client))
        } else {
            None
        };

        let eth_provider: continuity::rpc::SharedEthProvider =
            if let Some(ref archiver_url) = chain.archiver_url {
                debug!(
                    chain_key,
                    archiver_url = %redact_url_query(archiver_url),
                    "[startup] wrapping ETH client with archiver HTTP provider"
                );
                Arc::new(continuity::archiver::ArchiverEthProvider::new(
                    archiver_url.clone(),
                    eth_client,
                ))
            } else {
                eth_client
            };

        debug!(
            chain_key,
            "[startup] building ContinuityBuilder (continuity + CC3 + source chain providers)"
        );
        let builder = Arc::new(ContinuityBuilder::new_with_indexer(
            continuity_config,
            cc3_client.clone(),
            eth_provider,
            indexer_provider,
        ));

        checkpoint_intervals
            .write()
            .await
            .insert(chain_key, checkpoint_interval);

        Ok(builder)
    }

    pub async fn run(&self) -> Result<()> {
        let metrics: Metrics = self.prom_metrics.clone() as Metrics;

        let service = services::continuity_service::ContinuityService::new(
            self.builders.clone(),
            metrics.clone(),
            self.config.max_batch_size.get(),
        )
        .await?;

        let service = Arc::new(service);

        ProofGenMetrics::spawn_hardware_updater(self.prom_metrics.clone());

        let allowed: std::collections::HashSet<u64> = self.config.chain_keys();
        let app = build_app(service.clone(), allowed, self.prom_metrics.clone());
        let (http_shutdown_tx, http_shutdown_rx) = channel::<()>();

        let bind_host = &self.config.bind_host;
        let ip = bind_host.parse::<IpAddr>().with_context(|| {
            format!("Invalid bind host: '{bind_host}'. Expected IP address (e.g., '0.0.0.0', '127.0.0.1', '::1', '::')")
        })?;
        let bind_addr = SocketAddr::new(ip, self.config.bind_port);

        let server = run_http_server(app, bind_addr, http_shutdown_rx);
        tokio::pin!(server);

        info!("Server listening on {bind_addr}");

        let cc3_client_clone = self.cc3_client.clone();
        let checkpoint_intervals_clone = self.checkpoint_intervals.clone();
        let last_checkpoint_blocks_clone = self.last_checkpoint_blocks.clone();

        tokio::spawn(async move {
            if let Err(e) = events::start_cc3_event_subscription(
                cc3_client_clone,
                checkpoint_intervals_clone,
                last_checkpoint_blocks_clone,
                service,
            )
            .await
            {
                error!("CC3 event subscription failed: {e}");
            }
        });

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

    pub async fn get_checkpoint_interval(&self, chain_key: u64) -> Option<u64> {
        self.checkpoint_intervals
            .read()
            .await
            .get(&chain_key)
            .copied()
    }

    pub async fn get_last_checkpoint_block(&self, chain_key: u64) -> Option<u64> {
        self.last_checkpoint_blocks
            .read()
            .await
            .get(&chain_key)
            .copied()
    }
}

pub async fn shutdown_signal() {
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
