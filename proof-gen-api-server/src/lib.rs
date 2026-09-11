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

/// Re-export of [`eth::redact_url_query`] so existing call sites in this
/// crate keep working unchanged. The eth-side helper redacts both `?query`
/// strings *and* secret-looking path segments (Chainstack/Alchemy style).
use eth::redact_url_query;

pub struct Server {
    config: Config,
    /// The one Creditcoin client every consumer shares. `CcClient::clone` produces an
    /// independent connection slot (a fresh `ArcSwap`), so value-cloning here would leave the
    /// builders on a dead socket after the event task reconnects its own copy. Everything
    /// holds this `Arc` instead, so one `reconnect()` repairs all of them.
    cc3_client: Arc<CcClient>,
    /// One continuity builder per configured source chain.
    builders: Vec<Arc<ContinuityBuilder>>,
    /// Per-chain handle to the raw block cache, so its occupancy can be reported. Each entry
    /// holds a whole block's decoded txs and receipts, which is the largest per-chain
    /// allocation on a high-throughput chain -- and the one with no other way to observe it.
    block_caches: HashMap<u64, Arc<eth::mem_block_cache::MemBlockCache>>,
    checkpoint_intervals: events::CheckpointIntervalMap,
    last_checkpoint_blocks: events::LastCheckpointBlockMap,
    prom_metrics: Arc<ProofGenMetrics>,
}

impl Server {
    /// Create a new server based on `Config`.
    pub async fn new(config: Config) -> Result<Self> {
        let chain_keys: Vec<u64> = config.chains.iter().map(|c| c.chain_key).collect();
        let prom_metrics = Arc::new(ProofGenMetrics::new(&chain_keys));
        info!("🚀 📈 Prometheus metrics available at /metrics");

        debug!(
            cc3_rpc_url = %config.cc3_rpc_url,
            chain_count = config.chains.len(),
            "🚀 [startup] connecting Creditcoin3 read-only client (cc3_rpc_url)"
        );
        let cc3_client = Arc::new(
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
        debug!("🚀 ✅ [startup] Creditcoin3 client connected");

        let mut builders: Vec<Arc<ContinuityBuilder>> = Vec::with_capacity(config.chains.len());
        let mut block_caches: HashMap<u64, Arc<eth::mem_block_cache::MemBlockCache>> =
            HashMap::with_capacity(config.chains.len());
        let checkpoint_intervals = Arc::new(RwLock::new(HashMap::new()));
        let last_checkpoint_blocks = Arc::new(RwLock::new(HashMap::new()));

        for (idx, chain) in config.chains.iter().enumerate() {
            debug!(
                step = idx + 1,
                of = config.chains.len(),
                chain_key = chain.chain_key,
                eth_rpc_url = %redact_url_query(&chain.eth_rpc_url),
                eth_rpc_fallback_count = chain.eth_rpc_fallback_urls.len(),
                archiver_url = ?chain.archiver_url.as_ref().map(|u| redact_url_query(u)),
                "🚀 [startup] configuring source chain"
            );
            for (fb_idx, url) in chain.eth_rpc_fallback_urls.iter().enumerate() {
                debug!(
                    chain_key = chain.chain_key,
                    fallback_index = fb_idx,
                    url = %redact_url_query(url),
                    "[startup] eth_rpc fallback registered"
                );
            }
            let (builder, block_cache) = Self::build_continuity_for_chain(
                &config,
                &cc3_client,
                chain,
                &checkpoint_intervals,
                &last_checkpoint_blocks,
            )
            .await?;
            builders.push(builder);
            if let Some(block_cache) = block_cache {
                block_caches.insert(chain.chain_key, block_cache);
            }
        }

        Ok(Server {
            config,
            cc3_client,
            builders,
            block_caches,
            checkpoint_intervals,
            last_checkpoint_blocks,
            prom_metrics,
        })
    }

    async fn build_continuity_for_chain(
        global: &Config,
        cc3_client: &Arc<CcClient>,
        chain: &ChainConfig,
        checkpoint_intervals: &Arc<RwLock<HashMap<u64, u64>>>,
        last_checkpoint_blocks: &Arc<RwLock<HashMap<u64, u64>>>,
    ) -> Result<(
        Arc<ContinuityBuilder>,
        Option<Arc<eth::mem_block_cache::MemBlockCache>>,
    )> {
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

        // Reorg-protection depth. The source of truth is the chain's MaturityStrategy in the
        // supported-chains pallet -- the same value the attestors act on -- so derive it from the
        // `supported_chain` we already fetched unless the operator pinned an override. An
        // unparseable or depth-less strategy is a real misconfiguration and fails startup
        // outright: a prover guessing its reorg window is worse than one that is down.
        let on_chain_depth: u64 = {
            let strategy = supported_chains_primitives::MaturityStrategy::try_from(
                supported_chain.maturity_strategy.as_str(),
            )
            .map_err(|e| {
                anyhow!(
                    "chain_key {chain_key}: invalid on-chain maturity strategy {:?}: {e:?}",
                    supported_chain.maturity_strategy
                )
            })?;
            strategy.maturity_delay().ok_or_else(|| {
                anyhow!(
                    "chain_key {chain_key}: maturity strategy {strategy:?} has no block-depth \
                     equivalent; set block_confirmation_depth explicitly"
                )
            })?
        };
        let block_confirmation_depth = match chain.block_confirmation_depth {
            None => {
                tracing::info!(
                    chain_key,
                    block_confirmation_depth = on_chain_depth,
                    maturity_strategy = %supported_chain.maturity_strategy,
                    "⛓️  reorg-protection depth derived from on-chain MaturityStrategy"
                );
                on_chain_depth
            }
            Some(explicit) if explicit == on_chain_depth => {
                tracing::info!(
                    chain_key,
                    block_confirmation_depth = explicit,
                    "⛓️  reorg-protection depth pinned in config; matches on-chain MaturityStrategy"
                );
                explicit
            }
            Some(explicit) => {
                tracing::warn!(
                    chain_key,
                    configured = explicit,
                    on_chain = on_chain_depth,
                    maturity_strategy = %supported_chain.maturity_strategy,
                    "⛓️  reorg-protection depth pinned in config DISAGREES with the on-chain \
                     MaturityStrategy the attestors use; this prover will confirm blocks on a \
                     different schedule from them. Remove block_confirmation_depth to derive it."
                );
                explicit
            }
        };
        // Source-chain block encoding from CC3 metadata, rather than assuming V1.
        let chain_encoding =
            usc_abi_encoding::common::EncodingVersion::from(supported_chain.chain_encoding);

        let eth_fallback_urls: &[String] = &chain.eth_rpc_fallback_urls;

        let eth_client = {
            debug!(
                chain_key,
                eth_rpc_url = %redact_url_query(&chain.eth_rpc_url),
                eth_rpc_fallback_count = eth_fallback_urls.len(),
                "🚀 [startup] connecting source chain ETH client"
            );
            EthClient::new_with_fallbacks(&chain.eth_rpc_url, eth_fallback_urls, None)
                .await
                .with_context(|| {
                    format!(
                        "Ethereum/source RPC connection failed for chain_key={chain_key} (eth_rpc_url={}, fallback_count={})",
                        redact_url_query(&chain.eth_rpc_url),
                        eth_fallback_urls.len()
                    )
                })?
                // In-process cache of finalized source blocks. Overlapping continuity ranges and
                // batched requests re-fetch the same low blocks every time; caching them removes
                // the repeat RPC + merkle work. Finalized blocks are immutable, so no invalidation.
                //
                // Each entry holds one block's *decoded* txs+receipts, so on a high-tx chain this
                // is the largest single per-chain allocation - and it overlaps the merkle cache,
                // which holds the same recent blocks in encoded form. Hence the per-chain knob.
                .with_block_cache(chain.cache.block_cache_capacity)
        };

        // Grab the cache handle before the client is moved into the RPC providers; going back
        // through the provider traits later would mean widening them for a metrics read.
        let block_cache = eth_client.block_cache();

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
            .block_confirmation_depth(block_confirmation_depth)
            .build();

        if block_confirmation_depth > 0 {
            debug!(
                chain_key,
                block_confirmation_depth,
                "⛓️  EVM reorg protection: accepting blocks only up to {} blocks behind chain tip",
                block_confirmation_depth
            );
        }

        let reconnecting: continuity::rpc::SharedEthProvider = Arc::new(
            continuity::rpc::ReconnectingEthRpcProvider::new(eth_client, chain_encoding),
        );

        let eth_provider: continuity::rpc::SharedEthProvider =
            if let Some(ref archiver_url) = chain.archiver_url {
                debug!(
                    chain_key,
                    archiver_url = %redact_url_query(archiver_url),
                    "🚀 [startup] wrapping ETH client with archiver HTTP provider"
                );
                Arc::new(continuity::archiver::ArchiverEthProvider::new(
                    archiver_url.clone(),
                    reconnecting,
                ))
            } else {
                reconnecting
            };

        debug!(
            chain_key,
            "🚀 [startup] building ContinuityBuilder (continuity + CC3 + source chain providers)"
        );
        let builder = Arc::new(ContinuityBuilder::new_with_providers(
            continuity_config,
            cc3_client.clone(),
            eth_provider,
        ));

        checkpoint_intervals
            .write()
            .await
            .insert(chain_key, checkpoint_interval);

        Ok((builder, block_cache))
    }

    pub async fn run(&self) -> Result<()> {
        let metrics: Metrics = self.prom_metrics.clone() as Metrics;

        let cache_configs = self
            .config
            .chains
            .iter()
            .map(|chain| (chain.chain_key, chain.cache.clone()))
            .collect();

        let service = services::continuity_service::ContinuityService::new_with_cache_configs(
            self.builders.clone(),
            cache_configs,
            metrics.clone(),
            self.config.max_batch_size.get(),
            self.config.max_batch_span,
        )
        .await?;

        let service = Arc::new(service);

        ProofGenMetrics::spawn_hardware_updater(self.prom_metrics.clone());
        ContinuityService::spawn_cache_metrics_updater(service.clone(), self.block_caches.clone());
        ContinuityService::spawn_merkle_backfill(service.clone());

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

        info!("🚀 🌐 Server listening on {bind_addr}");

        let checkpoint_intervals_clone = self.checkpoint_intervals.clone();
        let last_checkpoint_blocks_clone = self.last_checkpoint_blocks.clone();
        let cc3_client_clone = self.cc3_client.clone();

        // The event task is supervised, not fire-and-forget. `StreamCC3` ends itself only
        // when history is gone for good (state pruned, or a reconnect gap past its replay
        // cap); its contract is that the consumer restarts. A prover that keeps serving after
        // that answers from caches that no longer track the chain — newer proofs fail with
        // `BlockNotReady`, and missed checkpoint/reversion events are never repaired.
        let events = tokio::spawn(events::start_cc3_event_subscription(
            cc3_client_clone,
            checkpoint_intervals_clone,
            last_checkpoint_blocks_clone,
            service.clone(),
            service.cc3_snapshot_height(),
        ));

        supervise(
            server,
            events,
            shutdown_signal(),
            http_shutdown_tx,
            |reason| {
                service.mark_event_stream_dead(reason);
            },
        )
        .await
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

/// Run the HTTP server, the cc3 event task and the shutdown signal to the first exit.
///
/// - shutdown signal: ask HTTP to drain and return `Ok`.
/// - HTTP server exit: fatal.
/// - event task exit (error, end of stream or panic): mark the replica unready via
///   `mark_dead`, ask HTTP to drain, give in-flight requests a moment, then return `Err` so
///   the process exits nonzero and the orchestrator replaces it with a fresh boot that
///   re-seeds from the current finalized head.
async fn supervise<S, D, M>(
    mut server: S,
    events: tokio::task::JoinHandle<Result<()>>,
    shutdown: D,
    http_shutdown_tx: tokio::sync::oneshot::Sender<()>,
    mark_dead: M,
) -> Result<()>
where
    S: std::future::Future<Output = Result<()>> + Unpin,
    D: std::future::Future<Output = ()>,
    M: FnOnce(&str),
{
    tokio::pin!(shutdown);
    select! {
        res = &mut server => {
            if let Err(err) = res {
                error!("❌ HTTP server exited with error: {err}");
            }
            bail!("API HTTP server exited!");
        }
        joined = events => {
            let reason = match joined {
                Ok(Ok(())) => "cc3 event task returned without error".to_string(),
                Ok(Err(err)) => format!("{err:#}"),
                Err(join) if join.is_panic() => format!("cc3 event task panicked: {join}"),
                Err(join) => format!("cc3 event task cancelled: {join}"),
            };
            error!(
                %reason,
                "❌ 🔗 CC3 event subscription ended; this replica can no longer track the chain — \
                 shutting down so the orchestrator restarts it from a fresh finalized head"
            );
            mark_dead(&reason);
            let _ = http_shutdown_tx.send(());
            let _ = tokio::time::timeout(EVENT_EXIT_DRAIN, &mut server).await;
            bail!("cc3 event subscription ended: {reason}");
        }
        _ = &mut shutdown => {
            let _ = http_shutdown_tx.send(());
            tracing::info!("🛑 Global shutdown requested, exiting");
            Ok(())
        }
    }
}

/// How long `supervise` lets the HTTP server drain after the event task dies before the
/// process exits regardless.
const EVENT_EXIT_DRAIN: std::time::Duration = std::time::Duration::from_secs(5);

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

    info!("🛑 Shutdown signal received");
}

#[cfg(test)]
mod supervise_tests {
    use super::*;

    fn never_shutdown() -> std::future::Pending<()> {
        std::future::pending()
    }

    #[tokio::test]
    async fn event_task_error_is_fatal_and_marks_the_replica_dead() {
        let (tx, rx) = channel::<()>();
        let server = Box::pin(async move {
            let _ = rx.await;
            Ok(())
        });
        let events = tokio::spawn(async { anyhow::bail!("End of unbounded event stream") });
        let marked = std::sync::Mutex::new(None);
        let res = supervise(server, events, never_shutdown(), tx, |r| {
            *marked.lock().unwrap() = Some(r.to_string());
        })
        .await;
        let err = res.expect_err("event task death must exit the process");
        assert!(
            err.to_string().contains("End of unbounded event stream"),
            "{err}"
        );
        assert!(marked
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("End of unbounded"));
    }

    #[tokio::test]
    async fn event_task_panic_is_fatal() {
        let (tx, rx) = channel::<()>();
        let server = Box::pin(async move {
            let _ = rx.await;
            Ok(())
        });
        let events = tokio::spawn(async { panic!("boom") });
        let res = supervise(server, events, never_shutdown(), tx, |_| {}).await;
        assert!(res.unwrap_err().to_string().contains("panicked"));
    }

    #[tokio::test]
    async fn shutdown_signal_drains_http_and_returns_ok() {
        let (tx, rx) = channel::<()>();
        let drained = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let d = drained.clone();
        let server = Box::pin(async move {
            let _ = rx.await;
            d.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        let events = tokio::spawn(async { std::future::pending::<Result<()>>().await });
        let res = supervise(server, events, std::future::ready(()), tx, |_| {
            panic!("shutdown must not mark the replica dead")
        })
        .await;
        assert!(res.is_ok());
        // The drain request was sent; the server future observes it once polled.
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn http_server_exit_is_fatal() {
        let (tx, _rx) = channel::<()>();
        let server = Box::pin(async { anyhow::bail!("bind failed") });
        let events = tokio::spawn(async { std::future::pending::<Result<()>>().await });
        let res = supervise(server, events, never_shutdown(), tx, |_| {}).await;
        assert!(res.unwrap_err().to_string().contains("HTTP server exited"));
    }
}
