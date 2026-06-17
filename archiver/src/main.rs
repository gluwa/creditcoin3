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

/// Resolve the parallelism for merkle root computation.
///
/// Merkle root computation is CPU-bound and runs on tokio's blocking thread
/// pool, so it is sized to the number of available CPU cores. Block fetching
/// (`max_fetch_tasks`) is async IO that does *not* occupy a core per task, so
/// it must NOT reduce this value — otherwise a high fetch concurrency starves
/// the merkle stage down to a single computation and caps overall throughput.
///
/// An explicit `--max-parallelism` override takes precedence when set.
fn compute_parallelism(override_value: Option<std::num::NonZeroUsize>) -> std::num::NonZeroUsize {
    override_value.unwrap_or_else(|| {
        std::thread::available_parallelism().unwrap_or(std::num::NonZeroUsize::new(4).unwrap())
    })
}

/// Out-of-order historical catch-up and gap backfill.
///
/// Fills every missing height in `[from, to]` — including interior gaps below
/// the current max stored height — by fetching at full concurrency and writing
/// roots to the store **as they complete** (no strict-order reordering), so
/// throughput is bounded by average RPC latency rather than the slowest
/// in-flight block. Missing heights are recomputed each pass via
/// [`RootStore::missing_ranges`], making it idempotent and restart-safe.
///
/// Runs repeated passes so a block that fails its client-internal retries on one
/// pass is retried on the next. Stops when the range is fully filled, when a
/// pass makes no progress (remaining heights are currently unfetchable — e.g. a
/// pruned/unavailable block), or after [`MAX_PASSES`](fast_catchup) passes.
async fn fast_catchup(
    store: &RootStore,
    client: &eth::Client,
    from: u64,
    to: u64,
    max_fetch_tasks: std::num::NonZeroUsize,
    flush_every: u64,
) -> Result<()> {
    /// Cap on gap-filling passes, so permanently-unfetchable blocks can't loop
    /// forever. Progress (≥1 block fetched) also resets nothing — each pass only
    /// targets what is still missing — so this is purely a backstop.
    const MAX_PASSES: usize = 6;

    if from > to {
        return Ok(());
    }

    for pass in 1..=MAX_PASSES {
        let missing = store.missing_ranges(from, to)?;
        let total_missing: u64 = missing.iter().map(|(s, e)| e - s + 1).sum();

        if total_missing == 0 {
            if pass == 1 {
                tracing::info!(
                    from,
                    to,
                    "fast-catchup: range already complete, nothing to do"
                );
            } else {
                tracing::info!(from, to, passes = pass - 1, "fast-catchup: all gaps filled");
            }
            return Ok(());
        }

        tracing::info!(
            pass,
            from,
            to,
            total_missing,
            tasks = max_fetch_tasks.get(),
            "fast-catchup: fetching missing heights"
        );

        let fetched = fast_catchup_pass(
            store,
            client,
            missing,
            total_missing,
            pass,
            max_fetch_tasks,
            flush_every,
        )
        .await?;

        tracing::info!(pass, fetched, total_missing, "fast-catchup: pass complete");

        if fetched == 0 {
            tracing::warn!(
                pass,
                remaining = total_missing,
                "fast-catchup: pass made no progress; remaining heights are currently unfetchable, stopping"
            );
            return Ok(());
        }
    }

    let remaining: u64 = store
        .missing_ranges(from, to)?
        .iter()
        .map(|(s, e)| e - s + 1)
        .sum();
    if remaining > 0 {
        tracing::warn!(
            remaining,
            max_passes = MAX_PASSES,
            "fast-catchup: gaps remain after max passes; re-run --fast-catchup to continue"
        );
    }
    Ok(())
}

/// A single fetch pass over the given `missing` ranges. Fetches each height at
/// full concurrency (one spawned task per fetch so body-read + deserialization
/// parallelize across worker threads) and writes roots as they land. Returns the
/// number of blocks successfully fetched and stored this pass; a block that
/// fails after the client's internal retries is logged and left for a later pass.
async fn fast_catchup_pass(
    store: &RootStore,
    client: &eth::Client,
    missing: Vec<(u64, u64)>,
    total_missing: u64,
    pass: usize,
    max_fetch_tasks: std::num::NonZeroUsize,
    flush_every: u64,
) -> Result<u64> {
    use futures::stream::StreamExt as _;

    let heights = missing.into_iter().flat_map(|(s, e)| s..=e);

    let mut fetches = futures::stream::iter(heights)
        .map(|height| {
            let client = client.clone();
            tokio::spawn(async move {
                match client.get_block(height, eth::EncodingVersion::V1).await {
                    Ok(block) => Some((block.number(), eth::simple_merkle_tree(&block).root())),
                    Err(e) => {
                        tracing::warn!(
                            height,
                            error = ?e,
                            "fast-catchup: block fetch failed after retries"
                        );
                        None
                    }
                }
            })
        })
        .buffer_unordered(max_fetch_tasks.get());

    let flush = flush_every.max(1) as usize;
    let mut batch_buf: Vec<(u64, sp_core::H256)> = Vec::with_capacity(flush);
    let mut done = 0u64;
    let start = Instant::now();
    let mut last_log = start;
    let mut last_done = 0u64;
    let mut recent_rate = 0.0f64;

    while let Some(joined) = fetches.next().await {
        // `joined` is the spawned task's `JoinHandle` result.
        let item = match joined {
            Ok(item) => item,
            Err(e) => {
                tracing::error!(error = %e, "fast-catchup: fetch task panicked or was cancelled");
                None
            }
        };
        let Some((height, root)) = item else {
            continue;
        };
        batch_buf.push((height, root));
        done += 1;

        if batch_buf.len() >= flush {
            store.put_roots(&batch_buf)?;
            batch_buf.clear();
        }

        if done % flush as u64 == 0 {
            let now = Instant::now();
            let window = now.duration_since(last_log).as_secs_f64();
            if window >= 1.0 {
                recent_rate = (done - last_done) as f64 / window;
                last_log = now;
                last_done = done;
            }
            tracing::info!(
                pass,
                done,
                total = total_missing,
                rate = format!("{recent_rate:.1} blocks/s"),
                remaining = total_missing.saturating_sub(done),
                "fast-catchup progress"
            );
        }
    }

    if !batch_buf.is_empty() {
        store.put_roots(&batch_buf)?;
    }
    store.flush().await?;

    Ok(done)
}

mod api;
mod compare;
mod config;
mod store;

use config::{Command, Config};
use store::RootStore;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cfg = Config::parse();

    // ── Subcommands ─────────────────────────────────────────────────────
    // Subcommands are one-shot tools that run and exit before the streaming
    // archiver starts. With no subcommand we fall through to streaming mode.
    if let Some(Command::Compare(args)) = &cfg.command {
        let mismatches = compare::run_compare(args)?;
        if mismatches > 0 {
            std::process::exit(1);
        }
        return Ok(());
    }

    // ── Streaming-mode argument validation ──────────────────────────────
    // `--rpc-ws` is modelled as optional so subcommands don't require it; in
    // streaming mode it is mandatory. `clap` already guarantees it is present
    // here (no subcommand was given), but unwrap defensively with a clear error.
    let rpc_ws = cfg
        .rpc_ws
        .clone()
        .ok_or_else(|| anyhow!("--rpc-ws <RPC_WS> is required to run the archiver"))?;

    // ── Storage ─────────────────────────────────────────────────────────
    if let Some(parent) = cfg.sled_db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let store = RootStore::open(&cfg.sled_db_path)?;

    // ── Determine resume height ─────────────────────────────────────────
    let latest_stored = store.latest_height()?;

    let mut start_height = match latest_stored {
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

    // ── Determine finalization lag ────────────────────────────────────────────────────
    // If a finalization lag parameter is passed, we use that. Otherwise fall back to
    // the lag from our on chain MaturityStrategy.
    let finaliztion_lag = if let Some(lag) = cfg.finalization_lag_override {
        tracing::info!(lag = lag, "Using cfg.finalization_lag_override");
        lag
    } else {
        // Fetch maturity strategy
        let lag = get_on_chain_finalization_lag(&cfg).await?;
        tracing::info!(lag = lag, "Using on chain lag from MaturityStrategy");
        lag
    };

    // ── Connect (WS primary + round-robin HTTP fetch pool) ──────────────────
    // The WS endpoint serves the new-head subscription and tip queries; the
    // HTTP endpoint(s) form a round-robin pool used for block fetching so the
    // fetch request load is spread evenly across them. Constructing the client
    // verifies every HTTP endpoint reports the same `chain_id` as the WS
    // endpoint and fails fast on a mismatch.
    let fetch_urls: Vec<String> = cfg.rpc_http.iter().map(|u| u.to_string()).collect();
    let stream_client =
        eth::Client::new_with_round_robin_fetch(rpc_ws.as_str(), &fetch_urls, None).await?;
    let chain_id = stream_client.chain_id();
    tracing::info!(
        chain_id,
        ws = %rpc_ws,
        fetch_endpoints = fetch_urls.len(),
        "connected to chain (WS primary + round-robin HTTP fetch pool)"
    );

    // ── Fast historical catch-up + gap backfill (out-of-order) ──────────
    // Bulk-fill every missing height from the configured `--start-height` up to
    // the finalized tip — including interior gaps below the current max stored
    // height — with out-of-order writes, before switching to the in-order
    // tip-following loop. Scanning from `cfg.start_height` (not the resume
    // point) is what makes this also a backfill: it recovers holes left by
    // earlier interrupted/erroring runs, not just heights above the max.
    if cfg.fast_catchup {
        let tip = stream_client.get_last_block().await?;
        // Fill up to the finalized tip, but never past an explicit `--end-height`
        // (the catch-up range is `[start, min(finalized tip, end)]`).
        let mut target = tip.saturating_sub(finaliztion_lag);
        if let Some(end) = cfg.end_height {
            target = target.min(end);
        }
        let catchup_from = cfg.start_height;

        if target >= catchup_from {
            fast_catchup(
                &store,
                &stream_client,
                catchup_from,
                target,
                cfg.max_fetch_tasks,
                cfg.flush_every.get(),
            )
            .await?;

            // Resume the live loop from the contiguous tip. Any heights still
            // missing after the bounded passes are recovered by re-running
            // `--fast-catchup`.
            start_height = store
                .latest_height()?
                .map(|h| h + 1)
                .unwrap_or(start_height);
            tracing::info!(
                resuming_from = start_height,
                "fast-catchup: handing off to live stream"
            );
        } else {
            tracing::info!(
                target,
                start_height = catchup_from,
                "fast-catchup: already at or past finalized tip, nothing to catch up"
            );
        }
    }

    // ── Backfill gaps ────────────────────────────────────────────────────
    if cfg.backfill {
        let gaps = store.find_gaps()?;
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

                let gap_config = stream_eth::roots::ConfigBuilder::new()
                    .with_client(stream_client.clone())
                    .with_start_height(*gap_start)
                    .with_finalization_lag(finaliztion_lag)
                    .with_max_concurrency(cfg.max_fetch_tasks)
                    .with_max_parallelism(compute_parallelism(cfg.max_parallelism))
                    .build();

                let mut gap_stream = stream_eth::StreamRoots::new(gap_config).await;
                let mut filled = 0u64;
                let flush_size = cfg.flush_every.get() as usize;
                let mut batch_buf = Vec::with_capacity(flush_size);

                while let Some(info) = gap_stream.next().await {
                    let done = info.height >= *gap_end;
                    batch_buf.push((info.height, info.root));
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

    // ── Root stream (with automatic reconnection) ───────────────────────
    let stream_config = stream_eth::roots::ConfigBuilder::new()
        .with_client(stream_client.clone())
        .with_start_height(start_height)
        .with_finalization_lag(finaliztion_lag)
        .with_max_concurrency(cfg.max_fetch_tasks)
        .with_max_parallelism(compute_parallelism(cfg.max_parallelism))
        .build();

    let mut root_stream = stream_eth::StreamRoots::new(stream_config).await;

    // ── Chain head tracker (for ETA) ───────────────────────────────────
    // Tip queries (`get_last_block`) go to the WS primary, so reuse the stream
    // client rather than opening a separate connection.
    let head_client = stream_client.clone();
    let current_head = head_client.get_last_block().await.unwrap_or(0);
    let chain_head = Arc::new(AtomicU64::new(current_head));
    {
        let head = chain_head.clone();
        let client = head_client.clone();
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

    // Recent-window throughput tracking. The cumulative average
    // (count / total_elapsed) is dragged down for a long time by a slow
    // startup, producing alarmingly pessimistic ETAs; instead we report the
    // rate over the most recent logging window so the figure reflects current
    // throughput.
    let mut last_log = start;
    let mut last_log_count = 0u64;
    let mut recent_rate = 0.0f64;

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

                    match eth::Client::new_with_round_robin_fetch(
                        rpc_ws.as_str(),
                        &fetch_urls,
                        None,
                    )
                    .await
                    {
                        Ok(new_client) => {
                            let new_config = stream_eth::roots::ConfigBuilder::new()
                                .with_client(new_client)
                                .with_start_height(resume_from)
                                .with_finalization_lag(finaliztion_lag)
                                .with_max_concurrency(cfg.max_fetch_tasks)
                                .with_max_parallelism(compute_parallelism(cfg.max_parallelism))
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
        last_height = Some(height);

        batch_buf.push((height, root));
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
            // Update the recent-window rate only when the window is wide enough
            // to be meaningful; logs can fire many times within a sub-second
            // burst as the reorder buffer drains, which would otherwise produce
            // wild rate spikes.
            let now = Instant::now();
            let window_secs = now.duration_since(last_log).as_secs_f64();
            if window_secs >= 1.0 {
                recent_rate = (count - last_log_count) as f64 / window_secs;
                last_log = now;
                last_log_count = count;
            }
            let rate = recent_rate;
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
    if remaining == 0 {
        return "synced".to_string();
    }
    // Rate not yet established (first window) — avoid implying we're caught up.
    if rate <= 0.0 {
        return "—".to_string();
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

async fn get_on_chain_finalization_lag(cfg: &Config) -> Result<u64> {
    // Check that chain_key is present in config
    let chain_key = if let Some(ck) = cfg.chain_key {
        ck
    } else {
        return Err(anyhow!(
            "Either cfg.finalization_lag_override or cfg.chain_key must be set!"
        ));
    };

    // Temp eth client for checking that chain_id matches expected.
    let primary_http = cfg
        .rpc_http
        .first()
        .ok_or_else(|| anyhow!("no --rpc-http endpoint configured"))?;
    let eth_client = eth::Client::new(primary_http.as_str(), None).await?;

    // Temp cc3 client for getting finalization lag
    let cc3_client =
        CcClient::new_read_only(&cfg.cc3_rpc_url)
            .await
            .with_context(|| {
                format!(
                    "Creditcoin3 RPC failed at cc3_rpc_url={}. \
                        Ensure the node is up, the URL scheme (ws/wss) matches, and network/firewall allows the connection.",
                    cfg.cc3_rpc_url
                )
            })?;

    let supported_chain = cc3_client
        .get_supported_chain(chain_key)
        .await
        .context("Failed to retrieve supported chain")?
        .ok_or(anyhow!(
            "No such supported chain. Check that provided chain_key is valid. chain_key: {}",
            chain_key
        ))?;

    if supported_chain.chain_id != eth_client.chain_id() {
        return Err(anyhow!(
            "Source chain id doesn't match registered id on Creditcoin under chain_key. chain_key: {}, creditcoin_source_id: {}, eth_rpc_source_id: {}",
            chain_key,
            supported_chain.chain_id,
            eth_client.chain_id()
        ));
    }

    let strategy_enum: supported_chains_primitives::MaturityStrategy = supported_chain
        .maturity_strategy
        .as_str()
        .try_into()
        .map_err(|e| anyhow!("Invalid maturity strategy: {:?}", e))?;

    // Return final maturity delay
    strategy_enum.maturity_delay().ok_or(anyhow!(
        "No maturity delay for strategy: strategy_enum: {:?}",
        strategy_enum
    ))
}
