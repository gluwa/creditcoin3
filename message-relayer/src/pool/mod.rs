//! Vote pool — the heart of relayer aggregation logic.
//!
//! Receives [`IndexedMessage`]s from the outbox watcher and [`MessageVote`]s from the libp2p
//! worker, then enforces PoC §6.2 validation rules (chain-first allowlist, ecrecover, signer
//! allowlist, dedup) before counting. When a `messageHash` accumulates `>= threshold` distinct
//! signers, the pool builds a [`DeliveryJob`] and dispatches it to the per-route delivery
//! channel.
//!
//! The pool runs as a single tokio task. State is **not** shared with other tasks — workers
//! talk to it strictly through mpsc channels. This keeps locking trivial and makes RAM-bound
//! invariants (PoC §9) easy to reason about.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use alloy::primitives::{Address, Signature, B256};
use anyhow::Result;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::VoteCacheConfig;
use crate::delivery::encode::encode_votes;
use crate::delivery::DeliveryJob;
use crate::events::IndexedMessage;
use crate::p2p::MessageVote;
use crate::prom::{Metrics, VoteOutcome};

/// Quorum: 2N/3 + 1 unique signers (PoC §6.3).
#[must_use]
pub fn calculate_threshold(n: usize) -> usize {
    (2 * n) / 3 + 1
}

/// Pre-resolved attester allowlist for a route. The runtime resolves [`AttesterSet`] (which may
/// be `OnChain`) into this concrete shape during `Server::new`, so the pool only deals with
/// EVM addresses + a fixed threshold.
///
/// [`AttesterSet`]: crate::config::AttesterSet
#[derive(Clone, Debug)]
pub struct RouteAttesters {
    pub chain_key: u64,
    pub attesters: Vec<Address>,
    pub threshold: usize,
}

/// Inputs / outputs for the pool task.
pub struct PoolHandles {
    pub indexed_rx: mpsc::Receiver<IndexedMessage>,
    pub vote_rx: mpsc::Receiver<MessageVote>,
    pub delivery_txs: HashMap<u64, mpsc::Sender<DeliveryJob>>,
}

/// Run the pool task. Returns when `cancel` fires or both inputs close.
pub async fn run(
    routes: Vec<RouteAttesters>,
    cache: VoteCacheConfig,
    handles: PoolHandles,
    metrics: Metrics,
    cancel: CancellationToken,
) -> Result<()> {
    let mut state = State::new(routes, cache);
    let PoolHandles {
        mut indexed_rx,
        mut vote_rx,
        delivery_txs,
    } = handles;

    let mut prune_tick = tokio::time::interval(Duration::from_secs(30));
    prune_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("🛑 vote pool exiting on cancel");
                return Ok(());
            }
            maybe = indexed_rx.recv() => {
                let Some(indexed) = maybe else {
                    info!("indexed_rx channel closed; shutting pool down");
                    return Ok(());
                };
                state.note_indexed(indexed, metrics.as_ref());
            }
            maybe = vote_rx.recv() => {
                let Some(vote) = maybe else {
                    info!("vote_rx channel closed; shutting pool down");
                    return Ok(());
                };
                if let Some(job) = state.note_vote(vote, metrics.as_ref()) {
                    if let Some(tx) = delivery_txs.get(&job.chain_key) {
                        if tx.send(job).await.is_err() {
                            warn!("delivery channel closed; dropping job");
                        }
                    } else {
                        warn!(chain_key = job.chain_key, "no delivery worker registered for chain_key");
                    }
                }
            }
            _ = prune_tick.tick() => {
                state.prune_expired();
                metrics.set_pool_messages_pending(state.total_pending() as i64);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct State {
    by_route: HashMap<u64, RouteState>,
    cache: VoteCacheConfig,
}

struct RouteState {
    chain_key: u64,
    attesters: Vec<Address>,
    threshold: usize,
    by_message: HashMap<B256, MessageSlot>,
    /// Insertion order, used together with [`MessageSlot::inserted_at`] for TTL/LRU eviction.
    order: VecDeque<B256>,
    cache_max: usize,
}

struct MessageSlot {
    indexed: IndexedMessage,
    signers: BTreeMap<Address, [u8; 65]>,
    delivered: bool,
    inserted_at: Instant,
}

impl State {
    fn new(routes: Vec<RouteAttesters>, cache: VoteCacheConfig) -> Self {
        let cap = cache.max_messages;
        Self {
            by_route: routes
                .into_iter()
                .map(|r| {
                    (
                        r.chain_key,
                        RouteState {
                            chain_key: r.chain_key,
                            attesters: r.attesters,
                            threshold: r.threshold,
                            by_message: HashMap::new(),
                            order: VecDeque::new(),
                            cache_max: cap,
                        },
                    )
                })
                .collect(),
            cache,
        }
    }

    fn note_indexed(&mut self, indexed: IndexedMessage, metrics: &dyn crate::prom::MetricsTrait) {
        let chain_key = indexed.chain_key;
        let Some(route) = self.by_route.get_mut(&chain_key) else {
            warn!(
                chain_key,
                "indexed message for unconfigured chain_key — dropping"
            );
            return;
        };
        let hash = indexed.message_hash;
        if route.by_message.contains_key(&hash) {
            // Re-org or duplicate finalized event; safe to ignore — keep the original slot.
            debug!(chain_key, %hash, "re-indexing existing message; keeping original slot");
            return;
        }
        route.by_message.insert(
            hash,
            MessageSlot {
                indexed,
                signers: BTreeMap::new(),
                delivered: false,
                inserted_at: Instant::now(),
            },
        );
        route.order.push_back(hash);
        route.evict_overflow();
        metrics.set_pool_messages_pending(self.total_pending() as i64);
    }

    fn note_vote(
        &mut self,
        vote: MessageVote,
        metrics: &dyn crate::prom::MetricsTrait,
    ) -> Option<DeliveryJob> {
        let chain_key = vote.chain_key;
        let route = self.by_route.get_mut(&chain_key)?;

        let hash = B256::from(vote.message_hash);
        let Some(slot) = route.by_message.get_mut(&hash) else {
            // PoC §6.2: chain-first allowlist — drop votes for messages we have not indexed.
            metrics.inc_vote(chain_key, VoteOutcome::Ignore);
            return None;
        };
        if slot.delivered {
            metrics.inc_vote(chain_key, VoteOutcome::Ignore);
            return None;
        }

        let claimed_signer = Address::from(vote.signer);

        // Allowlist check — cheap, do before `ecrecover`.
        if !route.attesters.contains(&claimed_signer) {
            metrics.inc_vote(chain_key, VoteOutcome::Reject);
            return None;
        }

        // Recover the actual signer and ensure it agrees with the claimed signer.
        let recovered = match recover_signer(&hash, &vote.signature) {
            Ok(addr) => addr,
            Err(err) => {
                debug!(%err, %claimed_signer, "ecrecover failed");
                metrics.inc_vote(chain_key, VoteOutcome::Reject);
                return None;
            }
        };
        if recovered != claimed_signer {
            metrics.inc_vote(chain_key, VoteOutcome::Reject);
            return None;
        }

        // Dedup.
        if slot.signers.contains_key(&recovered) {
            metrics.inc_vote(chain_key, VoteOutcome::Ignore);
            return None;
        }
        slot.signers.insert(recovered, vote.signature);
        metrics.inc_vote(chain_key, VoteOutcome::Accept);

        if slot.signers.len() < route.threshold {
            return None;
        }

        // Threshold reached — build a single DeliveryJob and mark delivered to ensure
        // idempotency. Subsequent votes for the same message will be ignored above.
        let signatures: Vec<[u8; 65]> = slot.signers.values().copied().collect();
        let signer_count = signatures.len();
        let votes_calldata = encode_votes(&signatures);
        slot.delivered = true;
        let elapsed = slot.inserted_at.elapsed();
        metrics.observe_votes_per_message(signer_count as u64);
        metrics.observe_time_to_threshold(elapsed);

        info!(
            chain_key,
            %hash,
            signer_count,
            elapsed_ms = elapsed.as_millis() as u64,
            "✅ threshold reached — dispatching delivery"
        );

        Some(DeliveryJob {
            chain_key,
            message_id: slot.indexed.message_id,
            emitter: slot.indexed.emitter,
            message_hash: hash,
            payload: slot.indexed.payload.clone(),
            votes_calldata,
            signer_count,
            indexed_at: slot.inserted_at,
        })
    }

    fn prune_expired(&mut self) {
        let ttl = Duration::from_secs(self.cache.ttl_seconds);
        let now = Instant::now();
        for route in self.by_route.values_mut() {
            route.prune_expired(now, ttl);
        }
    }

    fn total_pending(&self) -> usize {
        self.by_route.values().map(|r| r.by_message.len()).sum()
    }
}

impl RouteState {
    fn evict_overflow(&mut self) {
        while self.by_message.len() > self.cache_max {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.by_message.remove(&oldest);
        }
    }

    fn prune_expired(&mut self, now: Instant, ttl: Duration) {
        // Remove from front of order while expired and not delivered. Stop at the first
        // non-expired entry — the queue is roughly insertion-ordered.
        while let Some(front) = self.order.front().copied() {
            let Some(slot) = self.by_message.get(&front) else {
                self.order.pop_front();
                continue;
            };
            if slot.delivered {
                // Drop delivered entries eagerly; we keep idempotency only as long as the slot
                // exists, but TTL-based eviction will not push duplicates past the chain head.
                self.order.pop_front();
                self.by_message.remove(&front);
                continue;
            }
            if now.duration_since(slot.inserted_at) > ttl {
                self.order.pop_front();
                self.by_message.remove(&front);
                continue;
            }
            break;
        }
    }
}

fn recover_signer(hash: &B256, raw: &[u8; 65]) -> Result<Address> {
    let sig: Signature = raw[..]
        .try_into()
        .map_err(|e| anyhow::anyhow!("malformed signature bytes: {e}"))?;
    sig.recover_address_from_prehash(hash)
        .map_err(|e| anyhow::anyhow!("ecrecover failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prom::NoopMetrics;
    use alloy::primitives::address;

    fn route_for(chain_key: u64, attesters: Vec<Address>) -> RouteAttesters {
        let threshold = calculate_threshold(attesters.len());
        RouteAttesters {
            chain_key,
            attesters,
            threshold,
        }
    }

    fn indexed_for(chain_key: u64, hash: B256) -> IndexedMessage {
        IndexedMessage {
            chain_key,
            message_id: B256::from([7u8; 32]),
            emitter: address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            destination_chain_key: B256::from([0u8; 32]),
            creditcoin_chain_id: 1,
            payload: vec![1, 2, 3],
            message_hash: hash,
        }
    }

    #[test]
    fn threshold_two_thirds_plus_one() {
        assert_eq!(calculate_threshold(1), 1);
        assert_eq!(calculate_threshold(3), 3);
        assert_eq!(calculate_threshold(4), 3);
        assert_eq!(calculate_threshold(7), 5);
        assert_eq!(calculate_threshold(10), 7);
    }

    #[test]
    fn unknown_message_drops_vote_quietly() {
        let route = route_for(
            2,
            vec![address!("000000000000000000000000000000000000000a")],
        );
        let mut state = State::new(vec![route], VoteCacheConfig::default());
        let metrics = NoopMetrics::new();
        let vote = MessageVote {
            chain_key: 2,
            message_id: [7u8; 32],
            message_hash: [1u8; 32],
            signer: [0x0a; 20],
            signature: [0u8; 65],
        };
        assert!(state.note_vote(vote, metrics.as_ref()).is_none());
        assert_eq!(state.total_pending(), 0);
    }

    #[test]
    fn evicts_when_cap_reached() {
        let route = route_for(
            2,
            vec![address!("000000000000000000000000000000000000000a")],
        );
        let cache = VoteCacheConfig {
            ttl_seconds: 600,
            max_messages: 2,
        };
        let mut state = State::new(vec![route], cache);
        let metrics = NoopMetrics::new();
        for byte in 1u8..=4 {
            let mut h = [0u8; 32];
            h[0] = byte;
            state.note_indexed(indexed_for(2, B256::from(h)), metrics.as_ref());
        }
        assert_eq!(state.total_pending(), 2);
    }

    #[test]
    fn duplicate_indexed_is_idempotent() {
        let route = route_for(
            2,
            vec![address!("000000000000000000000000000000000000000a")],
        );
        let mut state = State::new(vec![route], VoteCacheConfig::default());
        let metrics = NoopMetrics::new();
        let hash = B256::from([1u8; 32]);
        state.note_indexed(indexed_for(2, hash), metrics.as_ref());
        state.note_indexed(indexed_for(2, hash), metrics.as_ref());
        assert_eq!(state.total_pending(), 1);
    }
}
