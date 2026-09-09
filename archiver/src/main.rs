//! The Archiver — continuously archives source chain data, computes merkle roots,
//! and serves root data over HTTP.
//!
//! Uses `stream_eth::StreamRoots` for block fetching with automatic RPC reconnection
//! and exponential backoff retries, ensuring gap-free data archival.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use cc_client::Client as CcClient;
use clap::Parser;
use futures::StreamExt;

/// Base delay between reconnection attempts (doubles each retry, capped at [`RECONNECT_MAX_DELAY`]).
const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(2);
/// Maximum delay between reconnection attempts.
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);

/// Compute parallelism for merkle root computation based on available CPUs
/// and how many threads are reserved for block fetching.
fn compute_parallelism(max_fetch_tasks: std::num::NonZeroUsize) -> std::num::NonZeroUsize {
    let available = std::thread::available_parallelism()
        .unwrap_or(std::num::NonZeroUsize::new(4).unwrap())
        .get();
    // Reserve threads for fetch tasks + 1 for the main loop, use the rest for computation.
    let parallelism = available.saturating_sub(max_fetch_tasks.get() + 1);
    // Defaults to at least 1 thread for computation.
    std::num::NonZeroUsize::new(parallelism).unwrap_or(std::num::NonZeroUsize::MIN)
}

mod api;
mod config;
mod store;

use config::Config;
use store::RootStore;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = Config::parse();

    // ── Storage ─────────────────────────────────────────────────────────
    if let Some(parent) = cfg.sled_db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let store = RootStore::open(&cfg.sled_db_path)?;

    // ── Determine resume height ─────────────────────────────────────────
    let latest_stored = store.latest_height()?;

    let start_height = match latest_stored {
        Some(latest) => {
            let resume = latest + 1;
            tracing::info!(
                stored = latest,
                total_entries = store.count(),
                resuming_from = resume,
                "resuming from database"
            );
            resume
        }
        None => {
            tracing::info!(from = cfg.start_height, "starting fresh (empty database)");
            cfg.start_height
        }
    };

    // Check if we've already passed the end height.
    if let Some(end) = cfg.end_height {
        if end < start_height {
            tracing::info!(
                end_height = end,
                start_height,
                "already archived past end-height, nothing to do"
            );
            return Ok(());
        }
    }

    // ── Source chain identity ───────────────────────────────────────────
    // Connect both RPC endpoints up front. They must agree on `chain_id`, and when
    // `CHAIN_KEY` is set that `chain_id` must be the one registered on Creditcoin for
    // this archiver's chain key. Both are fatal: archiving the wrong chain under a
    // given archive name silently corrupts every proof later built from it.
    let ws_client = eth::Client::new(cfg.rpc_ws.as_str(), None).await?;
    let http_client = eth::Client::new(cfg.rpc_http.as_str(), None).await?;
    if ws_client.chain_id() != http_client.chain_id() {
        return Err(anyhow!(
            "chain_id's from ws vs http don't match! ws_chain_id: {}, http_chain_id: {}",
            ws_client.chain_id(),
            http_client.chain_id(),
        ));
    }
    let source_chain_id = ws_client.chain_id();

    // ── Registered chain (Creditcoin) ───────────────────────────────────
    // Previously this lookup only ran when FINALIZATION_LAG was unset, so every
    // deployment that pinned the lag skipped the chain_id verification along with it.
    // The verification now runs whenever CHAIN_KEY is available.
    let on_chain_maturity = match cfg.chain_key {
        Some(chain_key) => {
            let cc3_client = CcClient::new_read_only(&cfg.cc3_rpc_url)
                .await
                .with_context(|| {
                    format!(
                        "Creditcoin3 RPC failed at cc3_rpc_url={}. \
                         Ensure the node is up, the URL scheme (ws/wss) matches, and network/firewall allows the connection.",
                        cfg.cc3_rpc_url
                    )
                })?;
            let chain = cc3_client
                .get_supported_chain(chain_key)
                .await
                .context("Failed to retrieve supported chain")?
                .ok_or_else(|| {
                    anyhow!(
                        "No such supported chain. Check that provided chain_key is valid. chain_key: {chain_key}"
                    )
                })?;
            let chain_name = String::from_utf8_lossy(&chain.chain_name).into_owned();

            if chain.chain_id != source_chain_id {
                return Err(anyhow!(
                    "source chain_id {} does not match the chain registered on Creditcoin under \
                     chain_key {} (chain_id {}, name {:?}); check RPC_HTTP/RPC_WS",
                    source_chain_id,
                    chain_key,
                    chain.chain_id,
                    chain_name,
                ));
            }
            tracing::info!(
                chain_key,
                chain_id = source_chain_id,
                name = %chain_name,
                "source chain verified against Creditcoin registration"
            );

            Some(on_chain_maturity(chain.maturity_strategy.as_str())?)
        }
        None => {
            tracing::warn!(
                chain_id = source_chain_id,
                "CHAIN_KEY not set: cannot verify that RPC_HTTP/RPC_WS serve the chain registered \
                 on Creditcoin; a misconfigured endpoint would be archived silently"
            );
            None
        }
    };

    // ── Determine maturity (fixed lag or RPC block tag) ─────────────────
    let maturity = resolve_maturity(cfg.finalization_lag_override, on_chain_maturity)?;

    // ── Backfill gaps ────────────────────────────────────────────────────
    if cfg.backfill {
        // Pass `cfg.start_height` so the gap-finder also reports a pre-first-stored gap
        // when the database begins at an intermediate height (e.g. partial snapshot
        // restore). Without an explicit anchor, `find_gaps` could only see neighbour-pair
        // gaps and would silently miss blocks below the first persisted entry.
        let gaps = store.find_gaps(Some(cfg.start_height))?;
        if gaps.is_empty() {
            tracing::info!("backfill: no gaps found");
        } else {
            let total_missing: u64 = gaps.iter().map(|(s, e)| e - s + 1).sum();
            tracing::info!(
                gaps = gaps.len(),
                total_missing,
                "backfill: found gaps, filling..."
            );

            for (gap_start, gap_end) in &gaps {
                tracing::info!(from = gap_start, to = gap_end, "backfill: filling gap");

                let ws_client = eth::Client::new(cfg.rpc_ws.as_str(), None).await?;
                let gap_config = stream_eth::roots::ConfigBuilder::new()
                    .with_client(ws_client)
                    .with_start_height(*gap_start)
                    .with_maturity(maturity)
                    .with_max_concurrency(cfg.max_fetch_tasks)
                    .with_max_parallelism(compute_parallelism(cfg.max_fetch_tasks))
                    .build();

                let mut gap_stream = stream_eth::StreamRoots::new(gap_config).await;
                let mut filled = 0u64;
                let flush_size = cfg.flush_every.get() as usize;
                let mut batch_buf = Vec::with_capacity(flush_size);

                while let Some(info) = gap_stream.next().await {
                    let done = info.height >= *gap_end;
                    // Store the source block hash alongside the root so canonical
                    // replacements (same root, different block) are reconciled across
                    // restart/backfill, not just within a single run.
                    batch_buf.push((info.height, info.root, info.hash));
                    filled += 1;

                    if batch_buf.len() >= flush_size || done {
                        store.put_roots(&batch_buf)?;
                        batch_buf.clear();
                    }

                    if filled % flush_size as u64 == 0 {
                        tracing::info!(
                            height = info.height,
                            filled,
                            remaining = gap_end.saturating_sub(info.height),
                            "backfill progress"
                        );
                    }

                    if done {
                        break;
                    }
                }

                store.flush().await?;
                tracing::info!(
                    from = gap_start,
                    to = gap_end,
                    filled,
                    "backfill: gap filled"
                );
            }

            tracing::info!("backfill complete");
        }
    }

    // ── Connect to chain ────────────────────────────────────────────────
    // Reuse the verified clients: WS for StreamRoots (subscriptions + block fetching),
    // HTTP for chain head tracking.
    tracing::info!(chain_id = source_chain_id, ws = %cfg.rpc_ws, http = %cfg.rpc_http, "connected to chain");

    // ── Root stream (with automatic reconnection) ───────────────────────
    let stream_config = stream_eth::roots::ConfigBuilder::new()
        .with_client(ws_client)
        .with_start_height(start_height)
        .with_maturity(maturity)
        .with_max_concurrency(cfg.max_fetch_tasks)
        .with_max_parallelism(compute_parallelism(cfg.max_fetch_tasks))
        .build();

    let mut root_stream = stream_eth::StreamRoots::new(stream_config).await;

    // ── Chain head tracker (for ETA) ───────────────────────────────────
    let current_head = http_client.get_last_block().await.unwrap_or(0);
    let chain_head = Arc::new(AtomicU64::new(current_head));
    {
        let head = chain_head.clone();
        let client = http_client.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(12)).await;
                if let Ok(h) = client.get_last_block().await {
                    head.store(h, Ordering::Release);
                }
            }
        });
    }

    tracing::info!(
        start = start_height,
        end_height = ?cfg.end_height,
        head = current_head,
        fetch_tasks = ?cfg.max_fetch_tasks,
        api = %cfg.api_bind,
        "starting archiver"
    );

    // ── HTTP API ────────────────────────────────────────────────────────
    let api_state = Arc::new(api::AppState {
        store: store.clone(),
        max_api_range: cfg.max_api_range,
    });

    let api_router = api::router(api_state);
    let listener = tokio::net::TcpListener::bind(cfg.api_bind).await?;
    tracing::info!(bind = %cfg.api_bind, "HTTP API listening");

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        axum::serve(listener, api_router)
            .with_graceful_shutdown(async {
                shutdown_rx.await.ok();
            })
            .await
            .ok();
    });

    // ── Ctrl+C handler ──────────────────────────────────────────────────
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        tracing::info!("shutting down...");
        let _ = cancel_tx.send(());
    });

    // ── Background flush task ───────────────────────────────────────────
    let flush_store = store.clone();
    let (flush_tx, mut flush_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        while flush_rx.recv().await.is_some() {
            if let Err(e) = flush_store.flush().await {
                tracing::error!("flush failed: {e}");
            }
        }
    });

    // ── Main loop ───────────────────────────────────────────────────────
    let mut count = 0u64;
    let start = Instant::now();
    let flush_size = cfg.flush_every.get() as usize;
    let mut batch_buf = Vec::with_capacity(flush_size);

    let stream_timeout = Duration::from_secs(cfg.stream_timeout_secs);
    let mut last_height: Option<u64> = None;

    loop {
        let next_item = tokio::select! {
            _ = &mut cancel_rx => break,
            result = tokio::time::timeout(stream_timeout, root_stream.next()) => result,
        };

        let info = match next_item {
            Ok(Some(info)) => info,
            reason => {
                let msg = match &reason {
                    Err(_) => "stalled (timeout)",
                    _ => "ended unexpectedly",
                };
                tracing::warn!(?last_height, reason = msg, "stream died, reconnecting...");

                // Flush any pending batch before reconnecting.
                if !batch_buf.is_empty() {
                    store.put_roots(&batch_buf)?;
                    batch_buf.clear();
                }

                // Reconnect with exponential backoff.
                let resume_from = last_height.map(|h| h + 1).unwrap_or(start_height);
                let mut delay = RECONNECT_BASE_DELAY;
                loop {
                    tokio::time::sleep(delay).await;
                    tracing::info!(resume_from, "attempting stream reconnection...");

                    match eth::Client::new(cfg.rpc_ws.as_str(), None).await {
                        Ok(new_ws) => {
                            let new_config = stream_eth::roots::ConfigBuilder::new()
                                .with_client(new_ws)
                                .with_start_height(resume_from)
                                .with_maturity(maturity)
                                .with_max_concurrency(cfg.max_fetch_tasks)
                                .with_max_parallelism(compute_parallelism(cfg.max_fetch_tasks))
                                .build();
                            root_stream = stream_eth::StreamRoots::new(new_config).await;
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("failed to connect WS client: {e}");
                        }
                    }

                    delay = (delay * 2).min(RECONNECT_MAX_DELAY);
                }
                continue;
            }
        };

        let height = info.height;
        let root = info.root;
        let block_hash = info.hash;
        last_height = Some(height);

        // Persist the source block hash with the root for reorg reconciliation.
        batch_buf.push((height, root, block_hash));
        count += 1;

        let end_reached = cfg.end_height.is_some_and(|end| height >= end);

        // Flush batch when full or at end.
        if batch_buf.len() >= flush_size || end_reached {
            store.put_roots(&batch_buf)?;
            batch_buf.clear();
        }

        // Stop if we've reached the end height.
        if end_reached {
            tracing::info!(height, total = count, "reached end height, stopping");
            break;
        }

        // Periodic flush + logging
        let is_flush = height % cfg.flush_every.get() == 0;
        let is_log = is_flush || count % cfg.flush_every.get() == 0;

        if is_flush {
            let _ = flush_tx.try_send(());
        }

        if is_log {
            let elapsed_secs = start.elapsed().as_secs_f64();
            let rate = if elapsed_secs > 0.0 {
                count as f64 / elapsed_secs
            } else {
                0.0
            };
            let target = cfg
                .end_height
                .unwrap_or_else(|| chain_head.load(Ordering::Acquire));
            let remaining = target.saturating_sub(height);
            let label = if is_flush { "flushed" } else { "✓" };
            tracing::info!(
                height,
                total = count,
                rate = format!("{rate:.1} blocks/s"),
                eta = format_eta(remaining, rate),
                behind = remaining,
                "{label}"
            );
        }
    }

    // Flush any remaining batch entries.
    if !batch_buf.is_empty() {
        store.put_roots(&batch_buf)?;
    }

    // ── Shutdown ────────────────────────────────────────────────────────
    tracing::info!("flushing final state...");
    store.flush().await?;
    let _ = shutdown_tx.send(());

    tracing::info!(
        total = count,
        elapsed = ?start.elapsed(),
        "archiver stopped"
    );

    Ok(())
}

fn format_eta(remaining: u64, rate: f64) -> String {
    if rate <= 0.0 || remaining == 0 {
        return "synced".to_string();
    }
    let secs = (remaining as f64 / rate) as u64;
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else {
        format!("{m}m")
    }
}

/// Maturity implied by an on-chain `MaturityStrategy` string: a fixed lag
/// (e.g. `"EvmFinalized"` → 64 blocks, `"FixedDelay: 5"` → 5) or an RPC block tag
/// (`"RpcSafe"` / `"RpcFinalized"`), resolved exactly as the attestors resolve it.
fn on_chain_maturity(maturity_strategy: &str) -> Result<eth::Maturity> {
    let strategy: supported_chains_primitives::MaturityStrategy = maturity_strategy
        .try_into()
        .map_err(|e| anyhow!("Invalid maturity strategy: {e:?}"))?;

    stream_eth::maturity_from_strategy(&strategy).ok_or_else(|| {
        anyhow!("Unsupported maturity strategy (no fixed delay and no RPC block tag): {strategy:?}")
    })
}

/// Pick the maturity. An explicit `FINALIZATION_LAG` always wins so operators keep an
/// escape hatch, but a value that disagrees with the on-chain registration is logged
/// loudly: the attestors follow the on-chain strategy, and a lag below theirs (or a fixed
/// lag where they follow a block tag) means the archiver flushes roots for blocks they do
/// not yet consider mature.
fn resolve_maturity(
    override_lag: Option<u64>,
    on_chain: Option<eth::Maturity>,
) -> Result<eth::Maturity> {
    match (override_lag, on_chain) {
        (Some(lag), Some(on_chain)) if on_chain != eth::Maturity::FixedLag(lag) => {
            tracing::warn!(
                lag,
                %on_chain,
                "FINALIZATION_LAG differs from the on-chain MaturityStrategy the attestors use"
            );
            Ok(eth::Maturity::FixedLag(lag))
        }
        (Some(lag), _) => {
            tracing::info!(lag, "Using cfg.finalization_lag_override");
            Ok(eth::Maturity::FixedLag(lag))
        }
        (None, Some(maturity)) => {
            tracing::info!(%maturity, "Using on-chain MaturityStrategy");
            Ok(maturity)
        }
        (None, None) => Err(anyhow!(
            "Either FINALIZATION_LAG or CHAIN_KEY (with CC3_RPC_URL) must be set"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eth::{BlockTag, Maturity};

    #[test]
    fn on_chain_maturity_follows_maturity_strategy() {
        assert_eq!(
            on_chain_maturity("EvmFinalized").unwrap(),
            Maturity::FixedLag(64)
        );
        assert_eq!(
            on_chain_maturity("EvmSafe").unwrap(),
            Maturity::FixedLag(32)
        );
        assert_eq!(
            on_chain_maturity("FixedDelay: 5").unwrap(),
            Maturity::FixedLag(5)
        );
        assert_eq!(
            on_chain_maturity("RpcSafe").unwrap(),
            Maturity::Tag(BlockTag::Safe)
        );
        assert_eq!(
            on_chain_maturity("RpcFinalized").unwrap(),
            Maturity::Tag(BlockTag::Finalized)
        );
    }

    #[test]
    fn on_chain_maturity_rejects_unknown_strategy() {
        assert!(on_chain_maturity("Bogus").is_err());
    }

    #[test]
    fn override_wins_even_when_it_disagrees_with_chain() {
        assert_eq!(
            resolve_maturity(Some(64), Some(Maturity::FixedLag(64))).unwrap(),
            Maturity::FixedLag(64)
        );
        assert_eq!(
            resolve_maturity(Some(10), Some(Maturity::FixedLag(64))).unwrap(),
            Maturity::FixedLag(10)
        );
        // A fixed override over a tag-following chain stays an escape hatch, but is warned.
        assert_eq!(
            resolve_maturity(Some(10), Some(Maturity::Tag(BlockTag::Safe))).unwrap(),
            Maturity::FixedLag(10)
        );
        assert_eq!(
            resolve_maturity(Some(0), None).unwrap(),
            Maturity::FixedLag(0)
        );
    }

    #[test]
    fn on_chain_maturity_used_without_override() {
        assert_eq!(
            resolve_maturity(None, Some(Maturity::FixedLag(5))).unwrap(),
            Maturity::FixedLag(5)
        );
        assert_eq!(
            resolve_maturity(None, Some(Maturity::Tag(BlockTag::Finalized))).unwrap(),
            Maturity::Tag(BlockTag::Finalized)
        );
    }

    #[test]
    fn neither_source_is_an_error() {
        assert!(resolve_maturity(None, None).is_err());
    }
}
