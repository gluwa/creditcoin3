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
use alloy::rpc::types::Filter;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

use write_ability::abi::IOutbox;
use write_ability::envelope::ReobservationRequest;
use write_ability::hash::message_hash;

use super::listener::IndexedMessage;
use super::resolver::ResolvedOutbox;

/// Minimum gap between honoring two reobservation requests for the *same* `message_id`. Bounds the
/// RPC work a spammy or adversarial requester can induce; a genuine stall lasts far longer than this,
/// so legitimate retries are unaffected.
pub const REOBS_MIN_INTERVAL: Duration = Duration::from_secs(30);
pub const REOBS_MAX_TRACKED_IDS: usize = 10_000;

/// Per-`message_id` cooldown tracker for reobservation requests. Synchronous and clock-injected so
/// it unit-tests without a network or real time.
#[derive(Default)]
pub struct ReobsRateLimiter {
    last: HashMap<B256, Instant>,
    order: VecDeque<B256>,
    min_interval: Duration,
    max_tracked: usize,
}

impl ReobsRateLimiter {
    #[must_use]
    pub fn new(min_interval: Duration) -> Self {
        Self {
            last: HashMap::new(),
            order: VecDeque::new(),
            min_interval,
            max_tracked: REOBS_MAX_TRACKED_IDS,
        }
    }

    #[must_use]
    pub fn with_capacity(min_interval: Duration, max_tracked: usize) -> Self {
        Self {
            last: HashMap::new(),
            order: VecDeque::new(),
            min_interval,
            max_tracked: max_tracked.max(1),
        }
    }

    /// Whether a request for `message_id` may be honored now. Returns `true` and records the time on
    /// the first call (or once the cooldown has elapsed); `false` while still cooling down. Also
    /// opportunistically forgets entries older than the cooldown so the map stays bounded.
    pub fn allow(&mut self, message_id: B256, now: Instant) -> bool {
        let allowed = self
            .last
            .get(&message_id)
            .is_none_or(|&t| now.duration_since(t) >= self.min_interval);
        if allowed {
            if !self.last.contains_key(&message_id) {
                self.order.push_back(message_id);
            }
            self.last.insert(message_id, now);
            let cutoff = self.min_interval;
            self.prune(now, cutoff);
        }
        allowed
    }

    fn prune(&mut self, now: Instant, cutoff: Duration) {
        self.last
            .retain(|_, &mut t| now.duration_since(t) < cutoff || t == now);
        self.order.retain(|id| self.last.contains_key(id));
        while self.last.len() > self.max_tracked {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.last.remove(&oldest);
        }
    }
}

/// Re-fetch and re-verify the message named by `request` against the resolved Outbox, returning the
/// [`IndexedMessage`] to re-sign — or `Ok(None)` when the request does not correspond to a genuine
/// `MessagePublished` we can confirm (forged / wrong block / wrong Outbox / `message_id` mismatch).
/// `Ok(None)` is deliberately not an error: an unverifiable request is simply ignored.
pub async fn reobserve<P: Provider>(
    provider: &P,
    resolved: &ResolvedOutbox,
    request: &ReobservationRequest,
) -> Result<Option<IndexedMessage>> {
    let requested_id = B256::from(request.message_id);

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

    for log in logs {
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
    fn first_request_is_allowed() {
        let mut rl = ReobsRateLimiter::new(REOBS_MIN_INTERVAL);
        assert!(rl.allow(id(1), Instant::now()));
    }

    #[test]
    fn repeat_inside_cooldown_is_denied() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        assert!(rl.allow(id(1), t0));
        assert!(!rl.allow(id(1), t0 + Duration::from_secs(5)));
        // A different message_id is independent.
        assert!(rl.allow(id(2), t0 + Duration::from_secs(5)));
    }

    #[test]
    fn allowed_again_after_cooldown() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        assert!(rl.allow(id(1), t0));
        assert!(rl.allow(id(1), t0 + Duration::from_secs(31)));
    }

    #[test]
    fn stale_entries_are_pruned() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(10));
        let t0 = Instant::now();
        rl.allow(id(1), t0);
        // A later allow for a different id prunes id(1) (older than the cooldown).
        rl.allow(id(2), t0 + Duration::from_secs(20));
        assert_eq!(rl.last.len(), 1, "stale entry should have been pruned");
    }

    #[test]
    fn capacity_evicts_oldest_distinct_ids() {
        let mut rl = ReobsRateLimiter::with_capacity(Duration::from_secs(30), 2);
        let now = Instant::now();
        assert!(rl.allow(id(1), now));
        assert!(rl.allow(id(2), now));
        assert!(rl.allow(id(3), now));

        assert!(!rl.last.contains_key(&id(1)), "oldest id should be evicted");
        assert!(rl.last.contains_key(&id(2)));
        assert!(rl.last.contains_key(&id(3)));
    }
}
