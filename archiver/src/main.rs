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
    // The verification now runs whenever CHAIN_KEY is available. The client is kept: in
    // tip-following mode it is what the archiver follows.
    let registered = match cfg.chain_key {
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

            Some((
                cc3_client,
                chain_key,
                chain.maturity_strategy.as_str().to_owned(),
            ))
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

    // ── Boundary ────────────────────────────────────────────────────────
    // Following the tip with CHAIN_KEY set, the archiver follows the latest attested height:
    // the attestors have already decided what is mature, and a cache that decides again can
    // only disagree with them. Explicit ranges and gap backfill still resolve maturity against
    // the source node, because they walk history that may have no attestations at all (the BSC
    // sweep computes roots for blocks the attestors will only reach later) and must not become
    // attestation-gated. That resolution is done lazily so a tip follower never needs to parse
    // the on-chain strategy.
    let mode = tip_mode(cfg.chain_key, cfg.end_height, cfg.finalization_lag_override);
    let source_maturity = || -> Result<eth::Maturity> {
        let on_chain = registered
            .as_ref()
            .map(|(_, _, strategy)| on_chain_maturity(strategy))
            .transpose()?;
        resolve_maturity(cfg.finalization_lag_override, on_chain)
    };

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
            let maturity = source_maturity()?;

            for (gap_start, gap_end) in &gaps {
                tracing::info!(from = gap_start, to = gap_end, "backfill: filling gap");

                let ws_client = eth::Client::new(cfg.rpc_ws.as_str(), None).await?;
                let gap_config = stream_eth::roots::ConfigBuilder::new()
                    .with_client(ws_client)
                    .with_start_height(*gap_start)
                    .with_bound(stream_eth::roots::Boundary::Source(maturity))
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
    let (boundary, attested) = match mode {
        TipMode::Attested { chain_key } => {
            let (cc3_client, _, _) = registered
                .as_ref()
                .expect("tip_mode only picks Attested when CHAIN_KEY is set");
            let rx = follow_attested_height(
                cc3_client.clone(),
                chain_key,
                Duration::from_secs(cfg.attested_poll_secs),
            );
            (stream_eth::roots::Boundary::Attested(rx.clone()), Some(rx))
        }
        TipMode::Source => (
            stream_eth::roots::Boundary::Source(source_maturity()?),
            None,
        ),
    };
    let stream_config = stream_eth::roots::ConfigBuilder::new()
        .with_client(ws_client)
        .with_start_height(start_height)
        .with_bound(boundary.clone())
        .with_max_concurrency(cfg.max_fetch_tasks)
        .with_max_parallelism(compute_parallelism(cfg.max_fetch_tasks))
        .build();

    let mut root_stream = stream_eth::StreamRoots::new(stream_config).await;

    // ── Chain head tracker (for ETA) ───────────────────────────────────
    // `chain_head` is 0 until the first successful read, and `head_seen_at` is the wall-clock
    // second of the last one, so consumers can tell a real head from "never read" or "stale
    // because HTTP has been failing"; see `known_head`.
    let current_head = http_client.get_last_block().await.unwrap_or(0);
    let chain_head = Arc::new(AtomicU64::new(current_head));
    let head_seen_at = Arc::new(AtomicU64::new(if current_head > 0 {
        now_secs()
    } else {
        0
    }));
    {
        let head = chain_head.clone();
        let seen_at = head_seen_at.clone();
        let client = http_client.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(HEAD_POLL_SECS)).await;
                if let Ok(h) = client.get_last_block().await {
                    head.store(h, Ordering::Release);
                    seen_at.store(now_secs(), Ordering::Release);
                }
            }
        });
    }

    tracing::info!(
        start = start_height,
        end_height = ?cfg.end_height,
        head = current_head,
        boundary = %boundary,
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
                // Under an attested boundary a quiet stream is the normal state between
                // attestations, not a stall: there is simply nothing released to fetch. Only
                // treat the timeout as a dead stream when the boundary has moved past what we
                // hold and the blocks still did not arrive.
                if let (Err(_), Some(rx)) = (&reason, &attested) {
                    let next_wanted = last_height.map(|h| h + 1).unwrap_or(start_height);
                    // The stream fetches up to the published bound clamped to the source head
                    // it has observed; judge the silence against the same number, or a node
                    // lagging the attestors would look like a stall and be torn down every
                    // timeout while it is healthy and simply behind.
                    // A head that was never read or has gone stale does not clamp: the stream
                    // is then judged against the raw bound, which errs towards a reconnect,
                    // never towards hiding a hung fetch behind "nothing to fetch".
                    let published = *rx.borrow();
                    let source_head = known_head(&chain_head, &head_seen_at);
                    let fetchable = fetchable_bound(published, source_head);
                    if fetchable.is_none_or(|bound| bound < next_wanted) {
                        tracing::info!(
                            ?published,
                            ?source_head,
                            next_wanted,
                            "nothing new to fetch (no new attestation, or the source node has \
                             not reached it yet)"
                        );
                        continue;
                    }
                }
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
                                .with_bound(boundary.clone())
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
        // Distance to whatever bounds this run: the explicit end, the attested height, or,
        // resolving maturity locally, the source head. Under an attested bound the last block
        // of every released range *is* the bound, and it is exactly the block the prover needs
        // next, so "at the tip" must be measured against that bound rather than the head.
        let target = cfg.end_height.unwrap_or_else(|| {
            let published = attested.as_ref().and_then(|rx| *rx.borrow());
            fetchable_bound(published, known_head(&chain_head, &head_seen_at))
                .unwrap_or_else(|| chain_head.load(Ordering::Acquire))
        });
        let remaining = target.saturating_sub(height);
        let at_tip = at_tip(remaining, cfg.flush_every);

        // Write the batch when full, at the end, or whenever we are at the tip: batching there
        // only delays when the API can serve a root that is already mature, which is what
        // proof-gen and the attestors are waiting on. Deep catch-up keeps the batched writes.
        if batch_buf.len() >= flush_size || end_reached || at_tip {
            store.put_roots(&batch_buf)?;
            batch_buf.clear();
        }

        // Stop if we've reached the end height.
        if end_reached {
            tracing::info!(height, total = count, "reached end height, stopping");
            break;
        }

        // Durability flush + logging: every block at the tip, every `flush_every` otherwise.
        let is_flush = at_tip || height % cfg.flush_every.get() == 0;
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

/// True when the archiver is close enough to the chain head that batching would delay the
/// visibility of mature roots: within one `flush_every` window of the target. At the tip
/// `remaining` settles at the finalization lag (plus a little head-tracker latency), which is
/// far below any sane `flush_every`, so tip-following writes and flushes every block without
/// operators having to set `FLUSH_EVERY=1`. If the head tracker has no value yet (`0`),
/// `remaining` is `0` and we err on the side of flushing.
fn at_tip(remaining: u64, flush_every: std::num::NonZeroU64) -> bool {
    remaining < flush_every.get()
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

/// How the tip-following stream bounds what it fetches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TipMode {
    /// Follow the latest attested height for `chain_key` on Creditcoin.
    Attested { chain_key: u64 },
    /// Resolve maturity against the source node: explicit ranges, or no `CHAIN_KEY` to follow.
    Source,
}

/// Following the tip with a chain key follows the attestations; everything else resolves
/// maturity locally. A `FINALIZATION_LAG` in attested mode has nothing to apply to and is
/// reported rather than silently dropped.
fn tip_mode(chain_key: Option<u64>, end_height: Option<u64>, override_lag: Option<u64>) -> TipMode {
    match (chain_key, end_height) {
        (Some(chain_key), None) => {
            if let Some(lag) = override_lag {
                tracing::warn!(
                    lag,
                    chain_key,
                    "FINALIZATION_LAG is ignored while following the tip: the archiver follows \
                     the latest attested height for this chain key. It still applies to explicit \
                     ranges (END_HEIGHT) and gap backfill."
                );
            }
            TipMode::Attested { chain_key }
        }
        _ => TipMode::Source,
    }
}

/// Publish the latest attested height for `chain_key` as a high-water mark, re-read every
/// `poll`. A read failure keeps the last value and repairs the Creditcoin connection before the
/// next read: the client is a value clone whose dead socket never heals on its own, so retrying
/// the same connection would freeze the bound until the process restarted. The bound can only
/// stall, never go backwards or invent progress.
fn follow_attested_height(
    cc3_client: CcClient,
    chain_key: u64,
    poll: Duration,
) -> tokio::sync::watch::Receiver<Option<u64>> {
    let (tx, rx) = tokio::sync::watch::channel(None);
    tokio::spawn(async move {
        loop {
            match cc3_client.fetch_last_finalized(chain_key).await {
                Ok(Some((height, _digest))) => {
                    tx.send_if_modified(|current| {
                        let advanced = advance_bound(current, height);
                        if advanced {
                            tracing::debug!(chain_key, height, "latest attested height");
                        } else if current.is_some_and(|c| height < c) {
                            // A lower reading is a revert or a lagging Creditcoin node. Neither
                            // may shrink the bound: roots for the higher range are already
                            // released, and stall detection and flush-at-tip both read this
                            // value. Reconciling roots that a revert orphaned is deliberately
                            // not this poller's job; it is surfaced here, and the archiver
                            // liveness stack's canonical-anchor check (#1347) drops an
                            // abandoned tail on the next start.
                            tracing::warn!(
                                chain_key,
                                height,
                                bound = ?current,
                                "attested height read below the published bound; keeping the bound"
                            );
                        }
                        advanced
                    });
                }
                Ok(None) => tracing::debug!(chain_key, "no attestation published yet"),
                Err(err) => {
                    tracing::warn!(
                        chain_key,
                        %err,
                        "could not read the latest attested height; keeping the last one and \
                         repairing the Creditcoin connection"
                    );
                    // `reconnect` swaps the shared connection atomically and carries its own
                    // backoff, so a Creditcoin outage costs one dial per poll, not a hot loop.
                    if let Err(err) = cc3_client.reconnect().await {
                        tracing::warn!(chain_key, %err, "Creditcoin reconnect failed");
                    }
                }
            }
            if tx.is_closed() {
                break;
            }
            tokio::time::sleep(poll).await;
        }
    });
    rx
}

/// Seconds between reads of the source head over HTTP (ETA and stall judgement only).
const HEAD_POLL_SECS: u64 = 12;
/// A head older than this many polls is treated as unknown rather than trusted.
const HEAD_STALE_AFTER: Duration = Duration::from_secs(HEAD_POLL_SECS * 3);

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The HTTP head tracker's value, if it can be trusted: read at least once and refreshed within
/// [`HEAD_STALE_AFTER`]. `None` otherwise, so callers fall back to not clamping.
fn known_head(head: &AtomicU64, seen_at: &AtomicU64) -> Option<u64> {
    head_if_fresh(
        head.load(Ordering::Acquire),
        seen_at.load(Ordering::Acquire),
        now_secs(),
        HEAD_STALE_AFTER,
    )
}

fn head_if_fresh(head: u64, seen_at: u64, now: u64, max_age: Duration) -> Option<u64> {
    (head > 0 && now.saturating_sub(seen_at) <= max_age.as_secs()).then_some(head)
}

/// What the stream can fetch up to: the published bound, clamped to the source head when one is
/// known. An unknown head does not clamp, so the judgement errs towards "the stream should have
/// progressed" (a reconnect) rather than towards hiding a stall.
fn fetchable_bound(published: Option<u64>, head: Option<u64>) -> Option<u64> {
    published.map(|bound| head.map_or(bound, |h| bound.min(h)))
}

/// Raise the published bound to `height` if that is an advance. Returns whether it moved. A
/// high-water mark by construction: equal and lower readings leave it untouched.
fn advance_bound(current: &mut Option<u64>, height: u64) -> bool {
    if current.is_none_or(|c| height > c) {
        *current = Some(height);
        true
    } else {
        false
    }
}

/// Pick a source-resolved maturity for the paths that still walk against the source node
/// (explicit ranges, gap backfill, tip-following without `CHAIN_KEY`). An explicit
/// `FINALIZATION_LAG` wins so operators keep an escape hatch there, but a value that disagrees
/// with the on-chain registration is logged loudly: a lag below the attestors' (or a fixed lag
/// where they follow a block tag) means roots for blocks they do not yet consider mature.
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
            "Either FINALIZATION_LAG or CHAIN_KEY (with CC3_RPC_URL) must be set to resolve \
             maturity for an explicit range, a gap backfill, or tip-following without a chain key"
        )),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn at_tip_within_one_flush_window_of_the_head() {
        let every = std::num::NonZeroU64::new(10_000).unwrap();
        // Deep catch-up keeps batching.
        assert!(!at_tip(45_000_000, every));
        assert!(!at_tip(10_000, every));
        // Inside the last window, and at the head itself (remaining == finalization lag).
        assert!(at_tip(9_999, every));
        assert!(at_tip(10, every));
        assert!(at_tip(0, every));
    }

    #[test]
    fn at_tip_with_flush_every_one_keeps_the_old_meaning() {
        let one = std::num::NonZeroU64::new(1).unwrap();
        assert!(at_tip(0, one));
        assert!(!at_tip(1, one));
    }

    #[test]
    fn missing_head_tracker_value_flushes_conservatively() {
        // chain_head defaults to 0 when the HTTP head fetch fails; remaining saturates to 0.
        let remaining = 0u64.saturating_sub(123_456);
        assert!(at_tip(
            remaining,
            std::num::NonZeroU64::new(10_000).unwrap()
        ));
    }

    use super::*;
    use eth::{BlockTag, Maturity};

    #[test]
    fn an_unread_or_stale_head_is_unknown_and_does_not_clamp() {
        let max = Duration::from_secs(36);
        assert_eq!(head_if_fresh(0, 0, 1_000, max), None, "never read");
        assert_eq!(head_if_fresh(500, 1_000, 1_000, max), Some(500));
        assert_eq!(
            head_if_fresh(500, 964, 1_000, max),
            Some(500),
            "at the edge"
        );
        assert_eq!(head_if_fresh(500, 963, 1_000, max), None, "stale");

        assert_eq!(fetchable_bound(None, Some(500)), None);
        assert_eq!(
            fetchable_bound(Some(600), Some(500)),
            Some(500),
            "lagging node clamps"
        );
        assert_eq!(fetchable_bound(Some(400), Some(500)), Some(400));
        assert_eq!(
            fetchable_bound(Some(600), None),
            Some(600),
            "unknown head must not hide a stall"
        );
    }

    #[test]
    fn the_attested_bound_only_ever_advances() {
        let mut bound = None;
        assert!(advance_bound(&mut bound, 100));
        assert_eq!(bound, Some(100));
        assert!(
            !advance_bound(&mut bound, 100),
            "equal reading is not a change"
        );
        assert!(
            !advance_bound(&mut bound, 90),
            "a lower reading must not shrink the bound"
        );
        assert_eq!(bound, Some(100));
        assert!(advance_bound(&mut bound, 130));
        assert_eq!(bound, Some(130));
    }

    #[test]
    fn following_the_tip_with_a_chain_key_follows_attestations() {
        assert_eq!(
            tip_mode(Some(8), None, None),
            TipMode::Attested { chain_key: 8 }
        );
        // A lag override has nothing to apply to here; it is warned about, not obeyed.
        assert_eq!(
            tip_mode(Some(8), None, Some(12)),
            TipMode::Attested { chain_key: 8 }
        );
    }

    #[test]
    fn explicit_ranges_and_keyless_runs_resolve_maturity_locally() {
        assert_eq!(tip_mode(Some(8), Some(1_000_000), None), TipMode::Source);
        assert_eq!(tip_mode(None, None, Some(12)), TipMode::Source);
        assert_eq!(tip_mode(None, Some(5), None), TipMode::Source);
    }

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
