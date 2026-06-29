//! Outbox event discovery.
//!
//! For every configured route, this module spawns a poller that watches the Creditcoin L1 EVM
//! endpoint for `MessagePublished` events on the route's resolved Outbox. New events become
//! [`IndexedMessage`]s pushed into the shared vote pool — the **chain-first allowlist** of PoC
//! §6.2: votes for `messageHash`es we have not indexed are dropped on arrival.

use std::sync::Arc;
use std::time::Duration;

use alloy::primitives::{Address, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use crate::abi::IOutbox;
use crate::checkpoint::CheckpointStore;
use crate::config::ChainRoute;
use crate::hash::message_hash;
use crate::prom::Metrics;
use write_ability::protocol::chain_key_to_bytes32;

pub mod factory;

pub use factory::{ConfigOverrideResolver, FactoryResolver, OutboxResolver};

/// Default poll cadence for `eth_getLogs`. WS subscription would be lower-latency but adds an
/// extra failure mode (silent stream stalls) we don't want in PoC scope.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 6;

/// A finalized message that the relayer has discovered on the Creditcoin Outbox. The vote pool
/// keys on `message_hash`; the rest of the fields are needed to recompute the calldata for
/// `Inbox.deliverMessage`.
#[derive(Clone, Debug)]
pub struct IndexedMessage {
    pub chain_key: u64,
    pub message_id: B256,
    pub emitter: Address,
    pub destination_chain_key: B256,
    pub creditcoin_chain_id: u64,
    pub payload: Vec<u8>,
    pub message_hash: B256,
}

/// Spawn one outbox watcher per route. Returns immediately; the watcher loops until `cancel`
/// fires or an unrecoverable error occurs.
#[allow(clippy::too_many_arguments)]
pub async fn watch_outbox(
    route: ChainRoute,
    creditcoin_eth_rpc_url: String,
    indexed_tx: mpsc::Sender<IndexedMessage>,
    metrics: Metrics,
    resolver: Arc<dyn OutboxResolver>,
    checkpoint: Option<Arc<CheckpointStore>>,
    cancel: CancellationToken,
) -> Result<()> {
    let chain_key = route.chain_key;
    let checkpoint_key = format!("outbox:{chain_key}");
    let provider = ProviderBuilder::new()
        .on_builtin(&creditcoin_eth_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: failed to connect to Creditcoin EVM RPC at {creditcoin_eth_rpc_url}"
            )
        })?;

    let outbox = resolver
        .resolve(&route)
        .await
        .with_context(|| format!("chain_key {chain_key}: outbox resolution failed"))?;

    // The destination chain_key is known locally — derived from the route's `u64` chain_key — and
    // bound into messageHash for every event seen on this outbox (see PoC §5.1). It is not read
    // back from the Outbox.
    let destination_chain_key = chain_key_to_bytes32(chain_key);

    let creditcoin_chain_id = provider.get_chain_id().await.with_context(|| {
        format!("chain_key {chain_key}: failed to read Creditcoin EVM chain id")
    })?;

    info!(
        chain_key,
        %outbox,
        ?destination_chain_key,
        creditcoin_chain_id,
        "📡 Outbox watcher initialized"
    );

    // Resume from the persisted cursor (so events emitted while we were down are not skipped),
    // falling back to the current head on first run / when persistence is disabled.
    let mut last_seen = match checkpoint.as_ref().and_then(|c| c.get(&checkpoint_key)) {
        Some(block) => {
            info!(
                chain_key,
                resume_from = block + 1,
                "↩️ resuming Outbox scan from checkpoint"
            );
            block
        }
        None => provider
            .get_block_number()
            .await
            .with_context(|| format!("chain_key {chain_key}: failed to read chain head"))?,
    };

    let mut tick = tokio::time::interval(Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(chain_key, "🛑 Outbox watcher exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                match poll_once(
                    chain_key,
                    outbox,
                    destination_chain_key,
                    creditcoin_chain_id,
                    route.block_confirmation_depth,
                    &provider,
                    &mut last_seen,
                    &indexed_tx,
                    metrics.as_ref(),
                    &cancel,
                ).await {
                    Ok(()) => {
                        if let Some(cp) = &checkpoint {
                            if let Err(err) = cp.set(&checkpoint_key, last_seen) {
                                warn!(chain_key, %err, "failed to persist Outbox checkpoint");
                            }
                        }
                    }
                    Err(err) => warn!(chain_key, %err, "outbox poll iteration failed; will retry"),
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn poll_once<P: Provider>(
    chain_key: u64,
    outbox: Address,
    destination_chain_key: B256,
    creditcoin_chain_id: u64,
    confirmation_depth: u64,
    provider: &P,
    last_seen: &mut u64,
    indexed_tx: &mpsc::Sender<IndexedMessage>,
    metrics: &dyn crate::prom::MetricsTrait,
    cancel: &CancellationToken,
) -> Result<()> {
    let tip = provider.get_block_number().await?;
    let to_block = tip.saturating_sub(confirmation_depth);
    if to_block <= *last_seen {
        return Ok(());
    }
    let from_block = *last_seen + 1;

    let filter = Filter::new()
        .address(outbox)
        .event_signature(IOutbox::MessagePublished::SIGNATURE_HASH)
        .from_block(from_block)
        .to_block(to_block);

    let logs = provider
        .get_logs(&filter)
        .await
        .with_context(|| format!("eth_getLogs from {from_block} to {to_block} failed"))?;

    for log in logs {
        match IOutbox::MessagePublished::decode_log(&log.inner, true) {
            Ok(decoded) => {
                let payload = decoded.data.payload.to_vec();
                let hash = message_hash(
                    decoded.data.messageId,
                    decoded.data.emitterAddress,
                    destination_chain_key,
                    creditcoin_chain_id,
                    &payload,
                );
                let indexed = IndexedMessage {
                    chain_key,
                    message_id: decoded.data.messageId,
                    emitter: decoded.data.emitterAddress,
                    destination_chain_key,
                    creditcoin_chain_id,
                    payload,
                    message_hash: hash,
                };
                debug!(
                    chain_key,
                    message_id = %indexed.message_id,
                    message_hash = %indexed.message_hash,
                    "📨 Indexed MessagePublished"
                );
                metrics.inc_messages_indexed(chain_key);
                // Bounded channel. Indexed messages are NOT re-gossiped (they come from chain
                // logs and the cursor advances past them), so they must not be dropped: block if
                // the pool is briefly saturated, but bail promptly on shutdown.
                tokio::select! {
                    res = indexed_tx.send(indexed) => {
                        if res.is_err() {
                            error!(chain_key, "vote pool channel closed — exiting watcher");
                            anyhow::bail!("vote pool channel closed");
                        }
                    }
                    () = cancel.cancelled() => {
                        debug!(chain_key, "cancel during indexed dispatch; stopping poll");
                        return Ok(());
                    }
                }
            }
            Err(err) => {
                warn!(chain_key, %err, "could not decode MessagePublished log; skipping");
            }
        }
    }

    *last_seen = to_block;
    Ok(())
}
