//! The Archiver — continuously archives source chain data, computes merkle roots,
//! and serves root data over HTTP.
//!
//! Uses `stream_eth::StreamRoots` for block fetching with automatic RPC reconnection
//! and exponential backoff retries, ensuring gap-free data archival.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Parser;
use futures::StreamExt;

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

    let first_stored = store.first_height()?;
    let start_height = match latest_stored {
        Some(latest) => {
            let total = first_stored.map(|f| latest - f + 1).unwrap_or(0);
            let resume = latest + 1;
            tracing::info!(
                stored = latest,
                first = ?first_stored,
                total_entries = total,
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

                let ws_client = eth::Client::new(cfg.rpc_ws.as_str(), None).await?;
                let gap_config = stream_eth::roots::ConfigBuilder::new()
                    .with_client(ws_client)
                    .with_start_height(*gap_start)
                    .with_max_concurrency(cfg.max_fetch_tasks)
                    .build();

                let mut gap_stream = stream_eth::StreamRoots::new(gap_config).await?;
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

    // ── Connect to chain ────────────────────────────────────────────────
    // WS client for StreamRoots (subscriptions + block fetching).
    let ws_client = eth::Client::new(cfg.rpc_ws.as_str(), None).await?;
    let chain_id = ws_client.chain_id();
    tracing::info!(chain_id, ws = %cfg.rpc_ws, http = %cfg.rpc_http, "connected to chain");

    // HTTP client for chain head tracking.
    let http_client = eth::Client::new(cfg.rpc_http.as_str(), None).await?;

    // ── Root stream (with automatic reconnection) ───────────────────────
    let stream_config = stream_eth::roots::ConfigBuilder::new()
        .with_client(ws_client)
        .with_start_height(start_height)
        .with_max_concurrency(cfg.max_fetch_tasks)
        .build();

    let mut root_stream = stream_eth::StreamRoots::new(stream_config).await?;

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
    /// Base delay between reconnection attempts (doubles each retry, capped at 60s).
    const RECONNECT_BASE_DELAY: Duration = Duration::from_secs(2);
    const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(60);

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
                                .with_max_concurrency(cfg.max_fetch_tasks)
                                .build();

                            match stream_eth::StreamRoots::new(new_config).await {
                                Ok(new_stream) => {
                                    root_stream = new_stream;
                                    tracing::info!(resume_from, "stream reconnected");
                                    break;
                                }
                                Err(e) => {
                                    tracing::warn!("failed to create stream: {e}");
                                }
                            }
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
