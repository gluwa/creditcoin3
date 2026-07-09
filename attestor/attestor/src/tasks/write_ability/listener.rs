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

/// Per-poll RPC deadline. A healthy `eth_getLogs`/`eth_blockNumber` returns in well under this; the
/// timeout exists so a *black-holed* connection (socket accepted but no response — the failure mode
/// a bare alloy WS provider can silently enter after its one-shot reconnect gives up) surfaces as an
/// error that counts toward the consecutive-failure budget instead of pending forever and wedging
/// the whole write-ability task (which shares this provider with the reobservation responder).
const POLL_TIMEOUT: Duration = Duration::from_secs(30);

/// Consecutive failed polls before the listener gives up and returns `Err`. The write-ability task
/// harvests that error and propagates it to the supervisor, which restarts the pod — rebuilding the
/// provider from scratch. This is the reconnect story for the write-ability EVM provider: unlike the
/// block-attestation path (wrapped in the reconnecting `eth::Client`), this provider is a bare alloy
/// connection whose pubsub service exits permanently after a single failed reconnect, so a routine
/// RPC blip would otherwise silently kill message voting for the process lifetime (C1). At the 6s
/// cadence this rides out ~1 minute of fast errors, or (with `POLL_TIMEOUT`) a few minutes of a
/// black-holed endpoint, before restarting — long enough to absorb a transient blip, short enough
/// that a dead provider does not strand quorum indefinitely.
const MAX_CONSECUTIVE_POLL_FAILURES: u32 = 10;

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
    start_block: Option<u64>,
    tx: mpsc::Sender<IndexedMessage>,
    token: CancellationToken,
) -> Result<()> {
    let mut last_seen = if let Some(start) = start_block {
        tracing::info!(
            start_block = start,
            "⏮️ no persisted Outbox cursor; starting message-attestation scan from configured block"
        );
        start.saturating_sub(1)
    } else {
        provider
            .get_block_number()
            .await
            .context("failed to read Creditcoin L1 chain head")?
    };

    let mut tick = tokio::time::interval(Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tracing::info!(
        outbox = %resolved.address,
        ?resolved.destination_chain_key,
        creditcoin_chain_id = resolved.creditcoin_chain_id,
        block_confirmation_depth,
        "📡 message-attestation Outbox listener online"
    );

    let mut consecutive_failures: u32 = 0;
    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 Outbox listener exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                // Bound each poll so a black-holed connection can't hang the loop indefinitely; a
                // timeout counts as a failure just like an RPC error.
                let outcome = match tokio::time::timeout(
                    POLL_TIMEOUT,
                    poll_once(provider, &resolved, block_confirmation_depth, &mut last_seen, &tx),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "outbox poll exceeded {POLL_TIMEOUT:?} — RPC unresponsive"
                    )),
                };

                match outcome {
                    Ok(()) => consecutive_failures = 0,
                    Err(err) => {
                        consecutive_failures += 1;
                        // Give up after a sustained failure run. Returning Err lets the
                        // write-ability task harvest it and restart the pod, which rebuilds this
                        // (non-self-healing) provider — the only reconnect path it has (C1).
                        if consecutive_failures >= MAX_CONSECUTIVE_POLL_FAILURES {
                            return Err(err).with_context(|| {
                                format!(
                                    "outbox poll failed {consecutive_failures} times in a row — \
                                     RPC connection is likely dead; restarting to rebuild it"
                                )
                            });
                        }
                        tracing::warn!(
                            %err,
                            consecutive_failures,
                            max = MAX_CONSECUTIVE_POLL_FAILURES,
                            "outbox poll iteration failed; will retry"
                        );
                    }
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
                // The log matched the MessagePublished topic filter but failed to decode. That is
                // not a per-message data issue — it means our IOutbox ABI does not match the
                // deployed contract, a systematic misconfiguration. Bail instead of skipping: we
                // return before advancing `last_seen`, so the caller retries this exact range
                // rather than silently stepping over an on-chain message that would then never be
                // indexed, signed, or gossiped. Re-processing the range's already-sent logs on
                // retry is harmless (the aggregator dedups by signer and the relayer dedups votes).
                return Err(err).with_context(|| {
                    format!(
                        "failed to decode a MessagePublished log (block {:?}, tx {:?}) — IOutbox ABI likely does not match the deployed Outbox",
                        log.block_number, log.transaction_hash
                    )
                });
            }
        }
    }

    *last_seen = to_block;
    Ok(())
}
