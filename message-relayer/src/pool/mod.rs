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

use alloy::primitives::{Address, PrimitiveSignature, B256};
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

/// Pre-resolved attestor allowlist for a route. The runtime resolves [`AttestorSet`] (which may
/// be `OnChain`) into this concrete shape during `Server::new`, so the pool only deals with
/// EVM addresses + a fixed threshold.
///
/// [`AttestorSet`]: crate::config::AttestorSet
#[derive(Clone, Debug)]
pub struct RouteAttestors {
    pub chain_key: u64,
    pub attestors: Vec<Address>,
    pub threshold: usize,
}

/// Inputs / outputs for the pool task.
pub struct PoolHandles {
    pub indexed_rx: mpsc::Receiver<IndexedMessage>,
    pub vote_rx: mpsc::Receiver<MessageVote>,
    pub delivery_txs: HashMap<u64, mpsc::Sender<DeliveryJob>>,
    /// Hot-reloaded attestor sets from the per-route on-chain watchers. Each update replaces a
    /// route's allowlist + threshold and re-evaluates its pending messages. Routes with a static
    /// set never send here.
    pub set_update_rx: mpsc::Receiver<RouteAttestors>,
}

/// Run the pool task. Returns when `cancel` fires or both inputs close.
pub async fn run(
    routes: Vec<RouteAttestors>,
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
        mut set_update_rx,
    } = handles;

    // Publish the starting allowlist sizes (static routes report their configured size; on-chain
    // routes start empty until their watcher resolves the set).
    state.report_set_sizes(metrics.as_ref());

    // Once every set-update sender is dropped (e.g. no on-chain routes, or all watchers exited),
    // `recv()` yields `None` forever; flip this off so the branch stops being polled.
    let mut set_updates_open = true;

    let mut prune_tick = tokio::time::interval(Duration::from_secs(30));
    prune_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("🛑 vote pool exiting on cancel");
                return Ok(());
            }
            maybe = set_update_rx.recv(), if set_updates_open => {
                let Some(update) = maybe else {
                    set_updates_open = false;
                    continue;
                };
                for job in state.apply_attestor_set(update, metrics.as_ref()) {
                    if let Some(tx) = delivery_txs.get(&job.chain_key) {
                        tokio::select! {
                            res = tx.send(job) => {
                                if res.is_err() {
                                    warn!("delivery channel closed; dropping job");
                                }
                            }
                            () = cancel.cancelled() => {
                                info!("🛑 vote pool exiting on cancel (mid set-reload dispatch)");
                                return Ok(());
                            }
                        }
                    }
                }
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
                        // Bounded channel. Delivery jobs must not be dropped, so block here if the
                        // worker is briefly saturated — but stay responsive to shutdown rather than
                        // wedging the whole pool (and every other route) on one slow destination.
                        tokio::select! {
                            res = tx.send(job) => {
                                if res.is_err() {
                                    warn!("delivery channel closed; dropping job");
                                }
                            }
                            () = cancel.cancelled() => {
                                info!("🛑 vote pool exiting on cancel (mid-dispatch)");
                                return Ok(());
                            }
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
    attestors: Vec<Address>,
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
    fn new(routes: Vec<RouteAttestors>, cache: VoteCacheConfig) -> Self {
        let cap = cache.max_messages;
        Self {
            by_route: routes
                .into_iter()
                .map(|r| {
                    (
                        r.chain_key,
                        RouteState {
                            attestors: r.attestors,
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
        if !route.attestors.contains(&claimed_signer) {
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

    /// Publish the current allowlist size of every route (called at startup and after a reload).
    fn report_set_sizes(&self, metrics: &dyn crate::prom::MetricsTrait) {
        for (chain_key, route) in &self.by_route {
            metrics.set_attestor_set_size(*chain_key, route.attestors.len() as i64);
        }
    }

    /// Apply a hot-reloaded attestor set + threshold for one route. Re-evaluates that route's
    /// not-yet-delivered messages against the **new** set/threshold: signatures from signers no
    /// longer in the set stop counting, and a lowered threshold (or a now-sufficient set) can push
    /// an already-collected message over quorum — those are returned as [`DeliveryJob`]s to dispatch.
    fn apply_attestor_set(
        &mut self,
        update: RouteAttestors,
        metrics: &dyn crate::prom::MetricsTrait,
    ) -> Vec<DeliveryJob> {
        let chain_key = update.chain_key;
        let Some(route) = self.by_route.get_mut(&chain_key) else {
            warn!(
                chain_key,
                "attestor-set update for unconfigured chain_key — ignoring"
            );
            return Vec::new();
        };

        let changed = route.attestors != update.attestors || route.threshold != update.threshold;
        route.attestors = update.attestors;
        route.threshold = update.threshold;
        metrics.set_attestor_set_size(chain_key, route.attestors.len() as i64);

        if !changed {
            return Vec::new();
        }
        metrics.inc_attestor_set_reload(chain_key);
        info!(
            chain_key,
            attestors = route.attestors.len(),
            threshold = route.threshold,
            "🔄 attestor set hot-reloaded"
        );

        // Clone the (small) allowlist so we can iterate `by_message` mutably alongside it.
        let attestors = route.attestors.clone();
        let threshold = route.threshold;
        let mut jobs = Vec::new();
        for (hash, slot) in route.by_message.iter_mut() {
            if slot.delivered {
                continue;
            }
            let valid: Vec<[u8; 65]> = slot
                .signers
                .iter()
                .filter(|(addr, _)| attestors.contains(addr))
                .map(|(_, sig)| *sig)
                .collect();
            if valid.len() < threshold {
                continue;
            }
            slot.delivered = true;
            let signer_count = valid.len();
            let votes_calldata = encode_votes(&valid);
            let elapsed = slot.inserted_at.elapsed();
            metrics.observe_votes_per_message(signer_count as u64);
            metrics.observe_time_to_threshold(elapsed);
            info!(
                chain_key,
                %hash,
                signer_count,
                "✅ threshold reached after set reload — dispatching delivery"
            );
            jobs.push(DeliveryJob {
                chain_key,
                message_id: slot.indexed.message_id,
                emitter: slot.indexed.emitter,
                message_hash: *hash,
                payload: slot.indexed.payload.clone(),
                votes_calldata,
                signer_count,
                indexed_at: slot.inserted_at,
            });
        }
        jobs
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
    let sig: PrimitiveSignature = raw[..]
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

    fn route_for(chain_key: u64, attestors: Vec<Address>) -> RouteAttestors {
        let threshold = calculate_threshold(attestors.len());
        RouteAttestors {
            chain_key,
            attestors,
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

    /// Seed a slot with `signers` already accepted (bypassing ecrecover) so we can exercise
    /// `apply_attestor_set`'s re-evaluation directly.
    fn seed_slot(state: &mut State, chain_key: u64, hash: B256, signers: &[Address]) {
        let metrics = NoopMetrics::new();
        state.note_indexed(indexed_for(chain_key, hash), metrics.as_ref());
        let slot = state
            .by_route
            .get_mut(&chain_key)
            .unwrap()
            .by_message
            .get_mut(&hash)
            .unwrap();
        for (i, a) in signers.iter().enumerate() {
            slot.signers.insert(*a, [i as u8 + 1; 65]);
        }
    }

    #[test]
    fn set_reload_lower_threshold_dispatches_pending() {
        let (a, b, c) = (
            Address::from([0xa; 20]),
            Address::from([0xb; 20]),
            Address::from([0xc; 20]),
        );
        let mut state = State::new(
            vec![RouteAttestors {
                chain_key: 2,
                attestors: vec![a, b, c],
                threshold: 3,
            }],
            VoteCacheConfig::default(),
        );
        let hash = B256::from([1u8; 32]);
        seed_slot(&mut state, 2, hash, &[a, b]); // 2 signers, below threshold 3 → not delivered

        // Threshold drops to 2: the already-collected slot now clears quorum and must dispatch.
        let jobs = state.apply_attestor_set(
            RouteAttestors {
                chain_key: 2,
                attestors: vec![a, b, c],
                threshold: 2,
            },
            NoopMetrics::new().as_ref(),
        );
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].signer_count, 2);
    }

    #[test]
    fn set_reload_removing_signer_revokes_its_vote() {
        let (a, b, c) = (
            Address::from([0xa; 20]),
            Address::from([0xb; 20]),
            Address::from([0xc; 20]),
        );
        let mut state = State::new(
            vec![RouteAttestors {
                chain_key: 2,
                attestors: vec![a, b, c],
                threshold: 3,
            }],
            VoteCacheConfig::default(),
        );
        let hash = B256::from([1u8; 32]);
        seed_slot(&mut state, 2, hash, &[a, b]);

        // Remove `b` and require 2: only `a` still counts (1 < 2), so nothing dispatches and the
        // slot stays open.
        let jobs = state.apply_attestor_set(
            RouteAttestors {
                chain_key: 2,
                attestors: vec![a, c],
                threshold: 2,
            },
            NoopMetrics::new().as_ref(),
        );
        assert!(jobs.is_empty());
        let slot = state
            .by_route
            .get(&2)
            .unwrap()
            .by_message
            .get(&hash)
            .unwrap();
        assert!(!slot.delivered);
    }

    #[test]
    fn set_reload_no_change_is_noop() {
        let a = Address::from([0xa; 20]);
        let mut state = State::new(
            vec![RouteAttestors {
                chain_key: 2,
                attestors: vec![a],
                threshold: 1,
            }],
            VoteCacheConfig::default(),
        );
        let jobs = state.apply_attestor_set(
            RouteAttestors {
                chain_key: 2,
                attestors: vec![a],
                threshold: 1,
            },
            NoopMetrics::new().as_ref(),
        );
        assert!(jobs.is_empty());
    }
}
