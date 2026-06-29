//! In-memory message-vote aggregator (confluence §7.3 A7 + A11 / §3.2, §5).
//!
//! Counts **unique** signers per `messageHash` until the 2N/3+1 threshold is reached, with the
//! anti-abuse properties from §5 baked in:
//!
//! * **Chain-first allowlist** — vote state is only allocated for a `messageHash` after the
//!   corresponding finalized `MessagePublished` has been observed on-chain ([`note_indexed`]).
//!   Votes for unknown hashes are dropped without allocating, so a peer cannot grow memory by
//!   gossiping votes for hashes that were never published.
//! * **Bounded memory** — a hard cap on distinct tracked hashes with LRU eviction of the
//!   least-recently-updated *incomplete* entry, plus TTL expiry of stale incomplete aggregates.
//! * **Dedup** — a signer's second vote for the same hash does not advance the count.
//!
//! Signer authorization (signer ∈ active attestor set) and signature verification happen in the
//! gossip layer before a vote reaches the aggregator; this structure only counts authorized,
//! verified, chain-seen votes. It is deliberately synchronous and clock-injected so it can be unit
//! tested without a network or real time.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use alloy::primitives::Address;

/// Result of offering a vote to the aggregator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoteOutcome {
    /// Vote counted. `reached_threshold` is true **only on the transition** that first meets the
    /// threshold, so the caller fires "ready for delivery" exactly once.
    Accepted { reached_threshold: bool },
    /// Signer already counted for this hash — ignored.
    Duplicate,
    /// `messageHash` has not been seen on-chain yet — dropped per the chain-first allowlist.
    NotIndexed,
}

struct Entry {
    signers: HashSet<Address>,
    inserted_at: Instant,
    last_update: Instant,
    completed: bool,
}

/// Per-`chain_key` vote aggregator. Keyed by `messageHash` (`[u8; 32]`).
pub struct VoteAggregator {
    threshold: usize,
    max_tracked: usize,
    ttl: Duration,
    entries: HashMap<[u8; 32], Entry>,
}

impl VoteAggregator {
    #[must_use]
    pub fn new(threshold: usize, max_tracked: usize, ttl: Duration) -> Self {
        Self {
            threshold: threshold.max(1),
            max_tracked: max_tracked.max(1),
            ttl,
            entries: HashMap::new(),
        }
    }

    /// Update the quorum threshold after an on-chain attestor-set change. Applies to all subsequent
    /// `add_vote` calls; already-`completed` entries are unaffected (they fired at the old quorum).
    pub fn set_threshold(&mut self, threshold: usize) {
        self.threshold = threshold.max(1);
    }

    /// Number of distinct hashes currently tracked.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.entries.len()
    }

    /// Unique signers counted for `hash` so far.
    #[must_use]
    pub fn signer_count(&self, hash: &[u8; 32]) -> usize {
        self.entries.get(hash).map_or(0, |e| e.signers.len())
    }

    /// Mark a `messageHash` as observed on-chain (chain-first allowlist). Allocates the entry so
    /// subsequent votes for it are counted. Idempotent. Enforces the tracked-hash cap by evicting
    /// the least-recently-updated incomplete entry.
    pub fn note_indexed(&mut self, hash: [u8; 32], now: Instant) {
        self.prune(now);
        if self.entries.contains_key(&hash) {
            return;
        }
        if self.entries.len() >= self.max_tracked {
            self.evict_one();
        }
        self.entries.insert(
            hash,
            Entry {
                signers: HashSet::new(),
                inserted_at: now,
                last_update: now,
                completed: false,
            },
        );
    }

    /// Offer a vote `(hash, signer)`. Only counts if `hash` is chain-seen and `signer` is new.
    pub fn add_vote(&mut self, hash: [u8; 32], signer: Address, now: Instant) -> VoteOutcome {
        self.prune(now);
        let Some(entry) = self.entries.get_mut(&hash) else {
            return VoteOutcome::NotIndexed;
        };
        if !entry.signers.insert(signer) {
            return VoteOutcome::Duplicate;
        }
        entry.last_update = now;
        let reached_threshold = !entry.completed && entry.signers.len() >= self.threshold;
        if reached_threshold {
            entry.completed = true;
        }
        VoteOutcome::Accepted { reached_threshold }
    }

    /// Evict the least-recently-updated incomplete entry. Falls back to any entry if all are
    /// complete (completed entries are kept only for dedup of late duplicates and are cheapest to
    /// drop). Called when at capacity.
    fn evict_one(&mut self) {
        let victim = self
            .entries
            .iter()
            .filter(|(_, e)| !e.completed)
            .min_by_key(|(_, e)| e.last_update)
            .map(|(k, _)| *k)
            .or_else(|| self.entries.keys().next().copied());
        if let Some(k) = victim {
            self.entries.remove(&k);
        }
    }

    /// Drop incomplete aggregates older than the TTL. Completed entries are retained (they protect
    /// against re-counting a late duplicate) until evicted by the cap.
    fn prune(&mut self, now: Instant) {
        let ttl = self.ttl;
        self.entries
            .retain(|_, e| e.completed || now.duration_since(e.inserted_at) <= ttl);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(b: u8) -> Address {
        Address::from([b; 20])
    }

    #[test]
    fn votes_for_unindexed_hash_are_dropped() {
        let mut agg = VoteAggregator::new(2, 100, Duration::from_secs(60));
        let now = Instant::now();
        assert_eq!(
            agg.add_vote([1u8; 32], addr(1), now),
            VoteOutcome::NotIndexed
        );
        assert_eq!(agg.tracked(), 0, "must not allocate state for unknown hash");
    }

    #[test]
    fn threshold_fires_exactly_once() {
        let mut agg = VoteAggregator::new(2, 100, Duration::from_secs(60));
        let now = Instant::now();
        let h = [9u8; 32];
        agg.note_indexed(h, now);
        assert_eq!(
            agg.add_vote(h, addr(1), now),
            VoteOutcome::Accepted {
                reached_threshold: false
            }
        );
        assert_eq!(
            agg.add_vote(h, addr(2), now),
            VoteOutcome::Accepted {
                reached_threshold: true
            }
        );
        // A third unique signer is still accepted but does not re-fire the threshold.
        assert_eq!(
            agg.add_vote(h, addr(3), now),
            VoteOutcome::Accepted {
                reached_threshold: false
            }
        );
    }

    #[test]
    fn duplicate_signer_does_not_advance() {
        let mut agg = VoteAggregator::new(2, 100, Duration::from_secs(60));
        let now = Instant::now();
        let h = [9u8; 32];
        agg.note_indexed(h, now);
        agg.add_vote(h, addr(1), now);
        assert_eq!(agg.add_vote(h, addr(1), now), VoteOutcome::Duplicate);
        assert_eq!(agg.signer_count(&h), 1);
    }

    #[test]
    fn cap_evicts_incomplete_lru() {
        let mut agg = VoteAggregator::new(2, 2, Duration::from_secs(600));
        let t0 = Instant::now();
        let (h1, h2, h3) = ([1u8; 32], [2u8; 32], [3u8; 32]);
        agg.note_indexed(h1, t0);
        agg.add_vote(h1, addr(1), t0); // h1 last_update = t0
        agg.note_indexed(h2, t0 + Duration::from_secs(1));
        agg.add_vote(h2, addr(1), t0 + Duration::from_secs(2)); // h2 last_update later
                                                                // At cap (2). Inserting h3 evicts the LRU incomplete entry — that's h1.
        agg.note_indexed(h3, t0 + Duration::from_secs(3));
        assert_eq!(agg.tracked(), 2);
        assert_eq!(agg.signer_count(&h1), 0, "h1 should have been evicted");
        assert_eq!(agg.signer_count(&h2), 1, "h2 retained");
    }

    #[test]
    fn ttl_expires_incomplete_entries() {
        let mut agg = VoteAggregator::new(3, 100, Duration::from_secs(10));
        let t0 = Instant::now();
        let h = [5u8; 32];
        agg.note_indexed(h, t0);
        agg.add_vote(h, addr(1), t0);
        // Advance past TTL; a later operation prunes the stale incomplete entry.
        let later = t0 + Duration::from_secs(11);
        assert_eq!(agg.add_vote(h, addr(2), later), VoteOutcome::NotIndexed);
        assert_eq!(agg.tracked(), 0);
    }

    #[test]
    fn completed_entry_survives_ttl_for_dedup() {
        let mut agg = VoteAggregator::new(1, 100, Duration::from_secs(10));
        let t0 = Instant::now();
        let h = [6u8; 32];
        agg.note_indexed(h, t0);
        // Threshold 1 → completes immediately.
        assert_eq!(
            agg.add_vote(h, addr(1), t0),
            VoteOutcome::Accepted {
                reached_threshold: true
            }
        );
        // Well past TTL, the completed entry is still there so a late duplicate is rejected.
        let later = t0 + Duration::from_secs(100);
        assert_eq!(agg.add_vote(h, addr(1), later), VoteOutcome::Duplicate);
    }
}
