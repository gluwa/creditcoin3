//! Creditcoin L1 Outbox event listener (confluence §7.3 A3 / §6.8).
//!
//! Polls `eth_getLogs` for `MessagePublished` on the resolved Outbox and emits an
//! [`IndexedMessage`] (with the canonical `messageHash` already computed) for each finalized event.
//!
//! Finality: the Outbox lives on Creditcoin L1, which has deterministic GRANDPA finality, so events
//! are surfaced up to the **finalized head** ([`FinalityPolicy::Finalized`]) — a finalized block
//! cannot be reorged out from under a signed vote (§6.8). `block_confirmation_depth` is only a
//! *fallback* probabilistic bound, used if finality stalls or the `finalized` tag is unavailable.
//! Polling (rather than `eth_subscribe`) avoids the silent-stream-stall failure mode, matching the
//! relayer.

use std::time::{Duration, Instant};

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::rpc::types::{BlockTransactionsKind, Filter};
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

/// Max block span per `eth_getLogs` request. The scan window can grow large — e.g. a long
/// Outbox-resolve wait leaves a wide gap between `last_seen` and the finalized tip on first poll —
/// and a single `eth_getLogs` over an unbounded span exceeds most RPC providers' range limits,
/// failing every poll until the failure budget restarts the task (which re-seeds from a fresh head
/// and permanently skips the unscanned span). Chunk the scan into ranges of this size instead; 2000
/// blocks is comfortably within the common provider caps (Alchemy/Infra allow ≥2k per query).
const MAX_LOG_BLOCK_RANGE: u64 = 2000;

/// How long the finalized head may stay frozen while the tip keeps advancing before we treat
/// finality as *stalled* (not merely lagging) and fall back to the probabilistic depth bound.
/// GRANDPA finality on Creditcoin normally lags the tip by seconds; a freeze this long means
/// finality is genuinely stuck, at which point signing must continue under the governed depth
/// bound rather than halt. Well above normal finality lag, well below any real outage budget.
const FINALITY_STALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The finality policy for the Outbox source chain (Creditcoin L1).
#[derive(Clone, Copy, Debug)]
pub enum FinalityPolicy {
    /// Sign up to the chain's GRANDPA-**finalized** head (exact, reorg-proof) — the production
    /// policy for Creditcoin. Falls back to `tip - fallback_depth` *only* when finality is
    /// unavailable or has stalled past [`FINALITY_STALL_TIMEOUT`] (governed probabilistic bound).
    Finalized { fallback_depth: u64 },
    /// Always sign up to `tip - depth` (probabilistic). For chains/harnesses without deterministic
    /// finality — e.g. the anvil unit-e2e, where the `finalized` tag has no GRANDPA meaning.
    Depth(u64),
}

/// Runtime finality state, tracked across polls to distinguish a genuine finality *stall* from
/// normal finality lag (see [`FinalityPolicy::Finalized`]).
#[derive(Clone, Copy, Debug)]
pub struct FinalityTracker {
    last_finalized: Option<u64>,
    last_advance: Instant,
    in_fallback: bool,
}

impl FinalityTracker {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            last_finalized: None,
            last_advance: now,
            in_fallback: false,
        }
    }
}

/// Decide the highest block to sign up to. Pure so the finality policy is unit-testable without an
/// RPC. Updates `tracker` (finalized-advance timestamp + whether we're in probabilistic fallback);
/// never regresses below the last known finalized head.
fn pick_to_block(
    finalized: Option<u64>,
    tip: u64,
    policy: &FinalityPolicy,
    tracker: &mut FinalityTracker,
    now: Instant,
) -> u64 {
    match *policy {
        FinalityPolicy::Depth(depth) => {
            tracker.in_fallback = false;
            tip.saturating_sub(depth)
        }
        FinalityPolicy::Finalized { fallback_depth } => match finalized {
            Some(f) => {
                let advanced = tracker.last_finalized.is_none_or(|prev| f > prev);
                if advanced {
                    tracker.last_finalized = Some(f);
                    tracker.last_advance = now;
                    tracker.in_fallback = false;
                    f
                } else if now.duration_since(tracker.last_advance) >= FINALITY_STALL_TIMEOUT {
                    // Finality genuinely stalled: sign the probabilistic bound, but never below the
                    // last finalized head we already trust.
                    tracker.in_fallback = true;
                    tip.saturating_sub(fallback_depth).max(f)
                } else {
                    // Finality is lagging but not stalled — stay at the finalized head.
                    tracker.in_fallback = false;
                    f
                }
            }
            None => {
                // Chain reports no finalized head (or the tag is unsupported / errored this poll).
                // Sign the probabilistic bound, but NEVER regress below the last finalized head we
                // already trusted: a transient `finalized`-read blip after a prior good read must not
                // let `tip - depth` advance the scan past a head we know is final and re-sign
                // not-yet-finalized logs (bugbot). `max(last_finalized)` mirrors the stall branch.
                tracker.in_fallback = true;
                tip.saturating_sub(fallback_depth)
                    .max(tracker.last_finalized.unwrap_or(0))
            }
        },
    }
}

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

    // Creditcoin L1 has deterministic GRANDPA finality, so the normal signing boundary is the
    // finalized head (reorg-proof); `block_confirmation_depth` is the governed probabilistic bound
    // used only if finality stalls or the `finalized` tag is unavailable (audit P1-2).
    let policy = FinalityPolicy::Finalized {
        fallback_depth: block_confirmation_depth,
    };
    let mut finality = FinalityTracker::new(Instant::now());

    tracing::info!(
        outbox = %resolved.address,
        ?resolved.destination_chain_key,
        creditcoin_chain_id = resolved.creditcoin_chain_id,
        fallback_depth = block_confirmation_depth,
        "📡 message-attestation Outbox listener online (signing finalized head; depth is fallback)"
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
                    poll_once(provider, &resolved, &policy, &mut finality, &mut last_seen, &tx),
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

/// Run a single poll iteration, signing up to the boundary chosen by `policy` (the finalized head,
/// or a probabilistic depth fallback — see [`pick_to_block`]). Exposed (beyond the internal
/// [`watch`] loop) so the anvil e2e test can drive polling deterministically.
pub async fn poll_once<P: Provider>(
    provider: &P,
    resolved: &ResolvedOutbox,
    policy: &FinalityPolicy,
    finality: &mut FinalityTracker,
    last_seen: &mut u64,
    tx: &mpsc::Sender<IndexedMessage>,
) -> Result<()> {
    let tip = provider.get_block_number().await?;

    // Read the finalized head only when the policy uses it. A finalized-tag read failure (node up
    // but the tag is unsupported/errored) is treated as "finalized unavailable" → depth fallback,
    // rather than failing the whole poll — the tip read above already covers a dead RPC.
    let finalized = match policy {
        FinalityPolicy::Finalized { .. } => {
            match provider
                .get_block_by_number(BlockNumberOrTag::Finalized, BlockTransactionsKind::Hashes)
                .await
            {
                Ok(Some(b)) => Some(b.header.number),
                Ok(None) => None,
                Err(err) => {
                    tracing::warn!(%err, "finalized-head read failed; using depth fallback this poll");
                    None
                }
            }
        }
        FinalityPolicy::Depth(_) => None,
    };

    let was_fallback = finality.in_fallback;
    let to_block = pick_to_block(finalized, tip, policy, finality, Instant::now());
    if finality.in_fallback && !was_fallback {
        tracing::warn!(
            tip,
            "⚠️ source finality stalled/unavailable — signing under the probabilistic depth fallback"
        );
    } else if !finality.in_fallback && was_fallback {
        tracing::info!("✅ source finality recovered — signing the finalized head again");
    }

    if to_block <= *last_seen {
        return Ok(());
    }

    // Chunk the scan into bounded block ranges (see `MAX_LOG_BLOCK_RANGE`). `last_seen` is
    // advanced after each *successful* chunk so progress is durable: a failure part-way through a
    // wide gap keeps everything already scanned, and a retry/restart resumes from there rather than
    // re-attempting (or skipping) the whole span.
    let mut from_block = *last_seen + 1;
    while from_block <= to_block {
        let chunk_to = to_block.min(from_block + MAX_LOG_BLOCK_RANGE - 1);
        scan_range(provider, resolved, from_block, chunk_to, tx).await?;
        *last_seen = chunk_to;
        from_block = chunk_to + 1;
    }
    Ok(())
}

/// Fetch + index `MessagePublished` logs in the inclusive block range `[from_block, to_block]`.
/// Returns `Err` (without the caller advancing `last_seen`) on an RPC failure or an ABI-mismatch
/// decode error, so the exact range is retried rather than stepped over.
async fn scan_range<P: Provider>(
    provider: &P,
    resolved: &ResolvedOutbox,
    from_block: u64,
    to_block: u64,
    tx: &mpsc::Sender<IndexedMessage>,
) -> Result<()> {
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(t0: Instant) -> FinalityTracker {
        FinalityTracker::new(t0)
    }

    #[test]
    fn depth_policy_uses_tip_minus_depth() {
        let t0 = Instant::now();
        let mut tr = tracker(t0);
        assert_eq!(
            pick_to_block(None, 110, &FinalityPolicy::Depth(3), &mut tr, t0),
            107
        );
        assert!(!tr.in_fallback);
        // depth 0 = index up to tip (the anvil e2e case).
        assert_eq!(
            pick_to_block(None, 110, &FinalityPolicy::Depth(0), &mut tr, t0),
            110
        );
    }

    #[test]
    fn finalized_primary_uses_finalized_head() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized { fallback_depth: 3 };
        let mut tr = tracker(t0);
        // First observation + subsequent advance both sign the finalized head, not tip-depth.
        assert_eq!(pick_to_block(Some(100), 110, &pol, &mut tr, t0), 100);
        assert!(!tr.in_fallback);
        assert_eq!(
            pick_to_block(Some(105), 120, &pol, &mut tr, t0 + Duration::from_secs(6)),
            105
        );
        assert!(!tr.in_fallback);
    }

    #[test]
    fn lagging_but_not_stalled_stays_at_finalized() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized { fallback_depth: 3 };
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(Some(100), 110, &pol, &mut tr, t0), 100);
        // Finalized frozen at 100 while tip climbs, but within the stall window → still 100.
        let within = t0 + FINALITY_STALL_TIMEOUT - Duration::from_secs(1);
        assert_eq!(pick_to_block(Some(100), 200, &pol, &mut tr, within), 100);
        assert!(!tr.in_fallback);
    }

    #[test]
    fn stalled_finality_falls_back_to_depth_bound() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized { fallback_depth: 3 };
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(Some(100), 110, &pol, &mut tr, t0), 100);
        // Finalized frozen past the stall timeout while tip advances → probabilistic bound,
        // never below the last finalized head.
        let past = t0 + FINALITY_STALL_TIMEOUT + Duration::from_secs(1);
        assert_eq!(pick_to_block(Some(100), 200, &pol, &mut tr, past), 197);
        assert!(tr.in_fallback);
        // Then finality recovers (advances) → back to signing the finalized head.
        assert_eq!(
            pick_to_block(Some(210), 220, &pol, &mut tr, past + Duration::from_secs(6)),
            210
        );
        assert!(!tr.in_fallback);
    }

    #[test]
    fn no_finalized_head_uses_depth_bound() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized { fallback_depth: 5 };
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(None, 100, &pol, &mut tr, t0), 95);
        assert!(tr.in_fallback);
    }

    #[test]
    fn no_finalized_head_never_regresses_below_last_finalized() {
        // A prior good read established finalized head 100; a later `finalized`-read blip (None) with
        // `tip - depth` = 96 must NOT regress below 100 and re-sign not-yet-finalized logs (bugbot).
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized { fallback_depth: 5 };
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(Some(100), 100, &pol, &mut tr, t0), 100);
        // Blip: no finalized head, tip advanced to 101 → tip-depth = 96, but clamp holds it at 100.
        assert_eq!(pick_to_block(None, 101, &pol, &mut tr, t0), 100);
        // Once tip-depth climbs past the last finalized head, the depth bound applies normally.
        assert_eq!(pick_to_block(None, 110, &pol, &mut tr, t0), 105);
    }
}
