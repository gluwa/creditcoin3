//! Reobservation responder (liveness recovery — solves the "relayer missed a vote" gap).
//!
//! Attestors normally sign each `MessagePublished` exactly once, when the [`listener`] surfaces it,
//! and never re-emit. If a relayer misses that one gossiped [`MessageVote`] (it was offline, or the
//! gossipsub window passed), the message can sit below quorum forever — there is no pull path.
//!
//! A [`ReobservationRequest`] is that pull path. A relayer that sees a message stalled below
//! threshold gossips one on [`reobservation_topic`](write_ability::protocol::reobservation_topic);
//! the [`p2p`](crate::tasks::p2p) task forwards it here. We do **not** trust the request: it is
//! unauthenticated, so before re-signing we independently re-fetch the named transaction from our
//! own Creditcoin RPC, confirm the `MessagePublished` for that `message_id` was emitted by the
//! resolved Outbox, and recompute the canonical `messageHash`. Only then do we re-sign and re-gossip
//! the same [`MessageVote`] we would have produced originally.
//!
//! The worst a forged or spammed request can do is make us perform a bounded `eth_getLogs` — bounded
//! because [`ReobsRateLimiter`] drops repeat requests for the same `message_id` inside a cooldown.
//!
//! [`listener`]: super::listener
//! [`MessageVote`]: write_ability::envelope::MessageVote
//! [`ReobservationRequest`]: write_ability::envelope::ReobservationRequest

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use alloy::primitives::B256;
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::rpc::types::{BlockTransactionsKind, Filter};
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

use write_ability::abi::IOutbox;
use write_ability::envelope::ReobservationRequest;
use write_ability::hash::message_hash;

use super::listener::IndexedMessage;
use super::resolver::ResolvedOutbox;

/// Cooldown after a *successfully* honored request for the same `message_id`. A genuine stall lasts
/// far longer than this, so legitimate relayer retries are unaffected while we avoid re-signing the
/// same message every few seconds.
pub const REOBS_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Cooldown after a *failed / unverifiable* request (forged block, wrong Outbox, decode miss). Much
/// shorter than the success interval: it exists only to bound the `eth_getLogs` an adversary can
/// induce by spamming garbage — it must NOT lock out the genuine relayer request for a full window
/// (see S2). A spammer targeting a stalled `message_id` can now only delay recovery by seconds, not
/// hold it below quorum indefinitely.
pub const REOBS_FAILURE_INTERVAL: Duration = Duration::from_secs(3);
pub const REOBS_MAX_TRACKED_IDS: usize = 10_000;

/// Per-request wall-clock deadline for the tip + `eth_getLogs` re-fetch. A black-holed RPC must not
/// let a single reobservation hang the responder; on timeout the request is dropped (the relayer
/// re-requests on its own cadence). Matches the listener's `POLL_TIMEOUT`.
pub const REOBS_RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-`message_id` cooldown tracker for reobservation requests. Synchronous and clock-injected so
/// it unit-tests without a network or real time.
///
/// The cooldown is applied *after* a request is honored (via [`record`](Self::record)), not at the
/// admission check ([`allow`](Self::allow)) — because requests are unauthenticated. If the check
/// itself recorded the full cooldown, a spammer replaying a forged request for a stalled
/// `message_id` would burn that message's window on every replay and starve the one genuine relayer
/// request that carries the correct block. Deferring the record — and using only a short cooldown on
/// failure — keeps the genuine request admissible within seconds.
#[derive(Default)]
pub struct ReobsRateLimiter {
    /// Per-`message_id` instant at which another request may next be honored (`now + cooldown`).
    next_allowed: HashMap<B256, Instant>,
    order: VecDeque<B256>,
    success_interval: Duration,
    failure_interval: Duration,
    max_tracked: usize,
}

impl ReobsRateLimiter {
    #[must_use]
    pub fn new(success_interval: Duration) -> Self {
        Self {
            next_allowed: HashMap::new(),
            order: VecDeque::new(),
            success_interval,
            failure_interval: REOBS_FAILURE_INTERVAL,
            max_tracked: REOBS_MAX_TRACKED_IDS,
        }
    }

    #[must_use]
    pub fn with_capacity(success_interval: Duration, max_tracked: usize) -> Self {
        Self {
            next_allowed: HashMap::new(),
            order: VecDeque::new(),
            success_interval,
            failure_interval: REOBS_FAILURE_INTERVAL,
            max_tracked: max_tracked.max(1),
        }
    }

    /// Whether a request for `message_id` may be honored at `now`. Pure check — does **not** record
    /// the cooldown (the caller does that via [`record`](Self::record) once the request has actually
    /// been verified). `true` when the id has never been seen or its cooldown has elapsed.
    #[must_use]
    pub fn allow(&self, message_id: B256, now: Instant) -> bool {
        self.next_allowed
            .get(&message_id)
            .is_none_or(|&next| now >= next)
    }

    /// Record that a request for `message_id` was honored at `now`, applying the success cooldown
    /// ([`REOBS_MIN_INTERVAL`]) when `verified` (a real `MessagePublished` was re-signed) or the
    /// short failure cooldown ([`REOBS_FAILURE_INTERVAL`]) otherwise. Opportunistically forgets
    /// entries whose cooldown has already elapsed so the map stays bounded.
    pub fn record(&mut self, message_id: B256, now: Instant, verified: bool) {
        let cooldown = if verified {
            self.success_interval
        } else {
            self.failure_interval
        };
        if !self.next_allowed.contains_key(&message_id) {
            self.order.push_back(message_id);
        }
        self.next_allowed.insert(message_id, now + cooldown);
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        // Drop ids whose cooldown has elapsed (their `next_allowed` is now in the past).
        self.next_allowed.retain(|_, &mut next| next > now);
        self.order.retain(|id| self.next_allowed.contains_key(id));
        while self.next_allowed.len() > self.max_tracked {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.next_allowed.remove(&oldest);
        }
    }
}

/// Whether `block_height` is final enough to re-sign, mirroring the exact trust envelope the Outbox
/// listener signs under ([`super::listener::pick_to_block`], audit P1-2): a block is trusted once it
/// is EITHER GRANDPA-finalized (`<= finalized`) OR buried under the governed probabilistic depth
/// (`+ confirmation_depth <= tip`). Pure so the gate is unit-testable without an RPC, and so it
/// can't silently drift from the listener's boundary.
fn reobs_final_enough(
    finalized: Option<u64>,
    tip: u64,
    block_height: u64,
    confirmation_depth: u64,
) -> bool {
    match finalized {
        Some(f) if block_height <= f => true,
        _ => block_height.saturating_add(confirmation_depth) <= tip,
    }
}

/// Re-fetch and re-verify the message named by `request` against the resolved Outbox, returning the
/// [`IndexedMessage`] to re-sign — or `Ok(None)` when the request does not correspond to a genuine
/// `MessagePublished` we can confirm (forged / wrong block / wrong Outbox / `message_id` mismatch).
/// `Ok(None)` is deliberately not an error: an unverifiable request is simply ignored.
pub async fn reobserve<P: Provider>(
    provider: &P,
    resolved: &ResolvedOutbox,
    confirmation_depth: u64,
    request: &ReobservationRequest,
) -> Result<Option<IndexedMessage>> {
    let requested_id = B256::from(request.message_id);

    // Finality gate — must accept exactly the trust envelope the listener signs under (P1-2), or
    // reobservation can't recover the same votes. A reobservation re-signs the exact vote the
    // listener would have produced, so it must not re-sign a message that isn't final yet: an
    // unauthenticated request could otherwise get us to sign a still-reorg-able publish, and quorum
    // signatures over a never-finalized message would satisfy the destination Inbox.
    //
    // The listener trusts a block once it is EITHER GRANDPA-finalized (`<= finalized head`, the
    // deterministic Creditcoin bound) OR buried under the governed probabilistic depth
    // (`+ confirmation_depth <= tip`, its fallback when the `finalized` tag is missing or finality
    // has stalled). Mirror that union here: gating on the finalized head alone would reject a block
    // the listener already signed via its depth fallback during a finality stall, stranding the
    // relayer's recovery of that message's votes. Both arms are within the operator's finality
    // safety bar (a not-yet-final request still falls through to `Ok(None)`); reobservation targets
    // stalled *old* messages, and the relayer re-requests on its own cadence anyway.
    let finalized = match provider
        .get_block_by_number(BlockNumberOrTag::Finalized, BlockTransactionsKind::Hashes)
        .await
    {
        Ok(Some(b)) => Some(b.header.number),
        _ => None,
    };
    let final_enough = match finalized {
        // Hot path: at or below the finalized head — deterministically final, no tip fetch needed.
        Some(f) if request.block_height <= f => true,
        // Above the finalized head (or no finalized tag): accept only if buried under the governed
        // probabilistic depth — the SAME fallback bound the listener signs under when finality
        // stalls or the tag is unavailable.
        _ => {
            let tip = provider
                .get_block_number()
                .await
                .context("reobservation tip fetch failed")?;
            reobs_final_enough(finalized, tip, request.block_height, confirmation_depth)
        }
    };
    if !final_enough {
        tracing::warn!(
            block = request.block_height,
            ?finalized,
            confirmation_depth,
            "🔎 reobservation request targets a not-yet-finalized block — ignoring"
        );
        return Ok(None);
    }

    // Tightly-scoped scan at the named block for our Outbox's MessagePublished — independent of the
    // request's claims beyond which block to look at.
    let filter = Filter::new()
        .address(resolved.address)
        .event_signature(IOutbox::MessagePublished::SIGNATURE_HASH)
        .from_block(request.block_height)
        .to_block(request.block_height);

    let logs = provider.get_logs(&filter).await.with_context(|| {
        format!(
            "reobservation eth_getLogs at block {} failed",
            request.block_height
        )
    })?;

    let requested_tx = B256::from(request.tx_hash);
    for log in logs {
        // Verify the log came from the transaction the request named (audit P3-1). The `tx_hash`
        // field previously rode along unchecked; enforce it so the re-signed vote is bound to the
        // exact emission the requester referenced. Safe now that we only act on finalized blocks —
        // a finalized transaction's hash is stable. (`message_id` + Outbox + block already pin the
        // message; this closes the "field implies a correlation the code doesn't perform" gap.)
        if log.transaction_hash != Some(requested_tx) {
            continue;
        }
        let Ok(decoded) = IOutbox::MessagePublished::decode_log(&log.inner, true) else {
            continue;
        };
        if decoded.data.messageId != requested_id {
            continue;
        }
        let payload = decoded.data.payload.to_vec();
        let hash = message_hash(
            decoded.data.messageId,
            decoded.data.emitterAddress,
            resolved.destination_chain_key,
            resolved.creditcoin_chain_id,
            &payload,
        );
        return Ok(Some(IndexedMessage {
            message_id: decoded.data.messageId,
            emitter: decoded.data.emitterAddress,
            payload,
            message_hash: hash,
        }));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> B256 {
        B256::from([b; 32])
    }

    #[test]
    fn final_enough_accepts_at_or_below_finalized_head() {
        // Deterministic-finality hot path: tip is irrelevant when block <= finalized.
        assert!(reobs_final_enough(Some(100), 100, 100, 5));
        assert!(reobs_final_enough(Some(100), 100, 99, 5));
    }

    #[test]
    fn final_enough_rejects_above_finalized_when_not_yet_deep() {
        // Above the finalized head and NOT yet buried by depth → not final enough.
        assert!(!reobs_final_enough(Some(100), 103, 101, 5));
    }

    #[test]
    fn final_enough_accepts_above_finalized_via_depth_fallback() {
        // Finality stalled at 100 but the listener kept signing via `tip - depth`; a reobservation
        // of a block above the stalled head must be recoverable once it is depth-buried (audit
        // P1-2: the two gates must share the same trust envelope). block 105, tip 120, depth 5 →
        // 105 + 5 <= 120.
        assert!(reobs_final_enough(Some(100), 120, 105, 5));
    }

    #[test]
    fn final_enough_uses_depth_when_no_finalized_tag() {
        assert!(reobs_final_enough(None, 120, 115, 5));
        assert!(!reobs_final_enough(None, 120, 116, 5));
    }

    #[test]
    fn first_request_is_allowed() {
        let rl = ReobsRateLimiter::new(REOBS_MIN_INTERVAL);
        assert!(rl.allow(id(1), Instant::now()));
    }

    #[test]
    fn allow_is_pure_and_does_not_record() {
        // Repeated `allow` without a `record` never denies — the admission check must not itself
        // impose a cooldown (that is what lets a forged request starve the genuine one; see S2).
        let rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        assert!(rl.allow(id(1), t0));
        assert!(rl.allow(id(1), t0 + Duration::from_secs(1)));
        assert!(rl.allow(id(1), t0 + Duration::from_secs(2)));
    }

    #[test]
    fn verified_request_uses_full_cooldown() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        assert!(rl.allow(id(1), t0));
        rl.record(id(1), t0, true);
        assert!(!rl.allow(id(1), t0 + Duration::from_secs(5)));
        assert!(rl.allow(id(1), t0 + Duration::from_secs(31)));
        // A different message_id is independent.
        assert!(rl.allow(id(2), t0 + Duration::from_secs(5)));
    }

    #[test]
    fn failed_request_uses_short_cooldown() {
        // A forged/unverifiable request only imposes the short failure cooldown, so the genuine
        // relayer request for the same id is admissible again within seconds — it can no longer be
        // held below quorum by a spammer replaying garbage inside a 30s window.
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        assert!(rl.allow(id(1), t0));
        rl.record(id(1), t0, false);
        assert!(
            !rl.allow(
                id(1),
                t0 + REOBS_FAILURE_INTERVAL - Duration::from_millis(1)
            ),
            "still cooling down within the (short) failure interval"
        );
        assert!(
            rl.allow(id(1), t0 + REOBS_FAILURE_INTERVAL),
            "admissible again after only the short failure cooldown — not the full 30s"
        );
    }

    #[test]
    fn stale_entries_are_pruned() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(10));
        let t0 = Instant::now();
        rl.record(id(1), t0, true);
        // A later record for a different id prunes id(1) (its 10s cooldown has elapsed).
        rl.record(id(2), t0 + Duration::from_secs(20), true);
        assert_eq!(
            rl.next_allowed.len(),
            1,
            "stale entry should have been pruned"
        );
    }

    #[test]
    fn capacity_evicts_oldest_distinct_ids() {
        let mut rl = ReobsRateLimiter::with_capacity(Duration::from_secs(30), 2);
        let now = Instant::now();
        rl.record(id(1), now, true);
        rl.record(id(2), now, true);
        rl.record(id(3), now, true);

        assert!(
            !rl.next_allowed.contains_key(&id(1)),
            "oldest id should be evicted"
        );
        assert!(rl.next_allowed.contains_key(&id(2)));
        assert!(rl.next_allowed.contains_key(&id(3)));
    }
}
