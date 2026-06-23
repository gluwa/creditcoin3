//! Creditcoin L1 Outbox event listener (confluence §7.3 A3 / §6.8).
//!
//! Polls `eth_getLogs` for `MessagePublished` on the resolved Outbox and emits an
//! [`IndexedMessage`] (with the canonical `messageHash` already computed) for each finalized event.
//!
//! Finality: events are only surfaced once they are `block_confirmation_depth` blocks below the
//! chain tip. That is the probabilistic-finality bound of §6.8 — signing from the unsafe head would
//! let honest attestors disagree after a reorg. Polling (rather than `eth_subscribe`) avoids the
//! silent-stream-stall failure mode, matching the relayer.

use std::time::Duration;

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::Filter;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use write_ability::abi::IOutbox;
use write_ability::hash::message_hash;

use super::resolver::ResolvedOutbox;

/// Poll cadence for `eth_getLogs`.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 6;

/// A finalized `MessagePublished` the attestor should vote on.
#[derive(Clone, Debug)]
pub struct IndexedMessage {
    pub message_id: B256,
    pub emitter: Address,
    pub payload: Vec<u8>,
    /// `keccak256(abi.encode(...))` — the digest the attestor signs (PoC §5.2).
    pub message_hash: B256,
}

/// Watch the resolved Outbox until `token` fires. Sends each finalized message on `tx`.
pub async fn watch<P: Provider>(
    provider: &P,
    resolved: ResolvedOutbox,
    block_confirmation_depth: u64,
    tx: mpsc::Sender<IndexedMessage>,
    token: CancellationToken,
) -> Result<()> {
    let mut last_seen = provider
        .get_block_number()
        .await
        .context("failed to read Creditcoin L1 chain head")?;

    let mut tick = tokio::time::interval(Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tracing::info!(
        outbox = %resolved.address,
        ?resolved.destination_chain_key,
        creditcoin_chain_id = resolved.creditcoin_chain_id,
        block_confirmation_depth,
        "📡 message-attestation Outbox listener online"
    );

    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 Outbox listener exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                if let Err(err) = poll_once(
                    provider,
                    &resolved,
                    block_confirmation_depth,
                    &mut last_seen,
                    &tx,
                ).await {
                    tracing::warn!(%err, "outbox poll iteration failed; will retry");
                }
            }
        }
    }
}

/// Run a single poll iteration over `(last_seen, tip - confirmation_depth]`. Exposed (beyond the
/// internal [`watch`] loop) so the anvil e2e test can drive polling deterministically.
pub async fn poll_once<P: Provider>(
    provider: &P,
    resolved: &ResolvedOutbox,
    confirmation_depth: u64,
    last_seen: &mut u64,
    tx: &mpsc::Sender<IndexedMessage>,
) -> Result<()> {
    let tip = provider.get_block_number().await?;
    let to_block = tip.saturating_sub(confirmation_depth);
    if to_block <= *last_seen {
        return Ok(());
    }
    let from_block = *last_seen + 1;

    let filter = Filter::new()
        .address(resolved.address)
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
                    resolved.destination_chain_key,
                    resolved.creditcoin_chain_id,
                    &payload,
                );
                let indexed = IndexedMessage {
                    message_id: decoded.data.messageId,
                    emitter: decoded.data.emitterAddress,
                    payload,
                    message_hash: hash,
                };
                tracing::debug!(
                    message_id = %indexed.message_id,
                    message_hash = %indexed.message_hash,
                    "📨 indexed finalized MessagePublished"
                );
                if tx.send(indexed).await.is_err() {
                    anyhow::bail!("message channel closed — listener exiting");
                }
            }
            Err(err) => {
                tracing::warn!(%err, "could not decode MessagePublished log; skipping");
            }
        }
    }

    *last_seen = to_block;
    Ok(())
}
