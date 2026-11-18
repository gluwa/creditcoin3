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
//! own Creditcoin RPC and confirm the `MessagePublished` for that `message_id` was emitted by the
//! Discovery-authorized Outbox at that source block. Later default changes, removal, and
//! replacement of Discovery do not invalidate this history. Only then do we re-sign (over
//! `message_id` itself — asc-contracts #54) and re-gossip
//! the same [`MessageVote`] we would have produced originally.
//!
//! The worst a forged or spammed request can do is make us perform one `eth_getLogs` filtered by
//! block, topic and `message_id` (a random id misses the block bloom and returns nothing), examine at
//! most [`REOBS_MAX_CANDIDATE_LOGS`] matching logs, and look up Outbox membership once per distinct
//! emitter. If the provider's log limit is hit we fetch only the *named* transaction's receipt, never
//! every receipt in the block. Repeats are bounded by [`ReobsRateLimiter`]: only a *verified*
//! request cools down its `message_id`; a failed one cools down just its own `(message_id, tx_hash,
//! block_height)`, so a forged request can never lock out the genuine one (CSUB-2138). [`ReobsQueue`]
//! drops queued duplicates and caps each relaying peer's share of the queue.
//!
//! [`listener`]: super::listener
//! [`MessageVote`]: write_ability::envelope::MessageVote
//! [`ReobservationRequest`]: write_ability::envelope::ReobservationRequest

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::{BlockNumberOrTag, Log};
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};
use parking_lot::Mutex;

use write_ability::abi::IOutbox;
use write_ability::envelope::ReobservationRequest;

use super::listener::{self, IndexedMessage, LogQuery};
use super::resolver::{self, ResolvedRoute};

/// Cooldown after a *successfully* honored request for the same `message_id`. A genuine stall lasts
/// far longer than this, so legitimate relayer retries are unaffected while we avoid re-signing the
/// same message every few seconds.
pub const REOBS_MIN_INTERVAL: Duration = Duration::from_secs(30);
/// Cooldown after a *failed / unverifiable* request (forged block, wrong Outbox, decode miss). It
/// only bounds the `eth_getLogs` an adversary can induce by replaying the same garbage, and is keyed
/// by the full request tuple ([`ReobsKey`]) so it cannot lock out a different, genuine request for
/// the same `message_id`.
pub const REOBS_FAILURE_INTERVAL: Duration = Duration::from_secs(3);
/// Capacity of each of the two cooldown maps (verified by `message_id`, failed by [`ReobsKey`]).
pub const REOBS_MAX_TRACKED_IDS: usize = 10_000;

/// Per-request wall-clock deadline for the whole re-verification, enforced by the worker via
/// [`settle_request`]. It is the only timeout: a black-holed RPC must not let a single reobservation
/// hang the responder, and per-call budgets would only add up to more than this. On timeout the
/// request is dropped and charged the failure cooldown (the relayer re-requests on its own cadence).
/// Matches the listener's `RPC_TIMEOUT`.
pub const REOBS_RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Most `MessagePublished` logs carrying the requested `message_id` we examine for the named
/// transaction. A genuine `message_id` is emitted exactly once, so more than this means a
/// log-stuffed transaction: the request is rejected rather than paying for a membership lookup per
/// log.
pub const REOBS_MAX_CANDIDATE_LOGS: usize = 16;

/// Queue capacity, shared with the other write-ability channels.
pub const REOBS_QUEUE_CAPACITY: usize = common::constants::CAPACITY_CHANNEL;
/// Queued requests one relaying peer may hold, so a single origin cannot fill the whole queue.
pub const REOBS_PER_PEER_QUEUE: usize = 10;

/// Identity of an unauthenticated request: every field an attacker controls. A failure cooldown is
/// keyed by all of it, so forging one field never blocks a request that differs in another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReobsKey {
    pub message_id: B256,
    pub tx_hash: B256,
    pub block_height: u64,
}

impl From<&ReobservationRequest> for ReobsKey {
    fn from(request: &ReobservationRequest) -> Self {
        Self {
            message_id: B256::from(request.message_id),
            tx_hash: B256::from(request.tx_hash),
            block_height: request.block_height,
        }
    }
}

/// Bounded `key -> instant at which it may next be honored` map with oldest-first eviction.
pub(crate) struct CooldownMap<K> {
    next_allowed: HashMap<K, Instant>,
    order: VecDeque<K>,
    max_tracked: usize,
}

impl<K: Copy + Eq + Hash> CooldownMap<K> {
    pub(crate) fn new(max_tracked: usize) -> Self {
        Self {
            next_allowed: HashMap::new(),
            order: VecDeque::new(),
            max_tracked: max_tracked.max(1),
        }
    }

    pub(crate) fn allow(&self, key: &K, now: Instant) -> bool {
        self.next_allowed.get(key).is_none_or(|&next| now >= next)
    }

    pub(crate) fn record(&mut self, key: K, now: Instant, cooldown: Duration) {
        if !self.next_allowed.contains_key(&key) {
            self.order.push_back(key);
        }
        self.next_allowed.insert(key, now + cooldown);
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        // Drop keys whose cooldown has elapsed (their `next_allowed` is now in the past).
        self.next_allowed.retain(|_, &mut next| next > now);
        self.order.retain(|key| self.next_allowed.contains_key(key));
        while self.next_allowed.len() > self.max_tracked {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.next_allowed.remove(&oldest);
        }
    }
}

/// Cooldown tracker for reobservation requests. Synchronous and clock-injected so it unit-tests
/// without a network or real time.
///
/// Requests are unauthenticated, so the two outcomes are tracked in **separate** maps:
/// * `verified` — keyed by `message_id`. Only a request that matched a real `MessagePublished` can
///   write here, so an attacker cannot use it to block a message.
/// * `failed` — keyed by the whole [`ReobsKey`]. A forged request (wrong tx/block for a real
///   `message_id`) cools down only itself; the genuine request has a different key and stays
///   admissible. Flooding it with random keys can only evict other failure entries, never a
///   `verified` one.
///
/// The cooldown is applied *after* a request is verified (via [`record_verified`] /
/// [`record_failed`]), not at the admission check ([`allow`]).
///
/// [`record_verified`]: Self::record_verified
/// [`record_failed`]: Self::record_failed
/// [`allow`]: Self::allow
pub struct ReobsRateLimiter {
    verified: CooldownMap<B256>,
    failed: CooldownMap<ReobsKey>,
    success_interval: Duration,
    failure_interval: Duration,
}

impl ReobsRateLimiter {
    #[must_use]
    pub fn new(success_interval: Duration) -> Self {
        Self::with_capacity(success_interval, REOBS_MAX_TRACKED_IDS)
    }

    #[must_use]
    pub fn with_capacity(success_interval: Duration, max_tracked: usize) -> Self {
        Self {
            verified: CooldownMap::new(max_tracked),
            failed: CooldownMap::new(max_tracked),
            success_interval,
            failure_interval: REOBS_FAILURE_INTERVAL,
        }
    }

    /// Whether `key` may be honored at `now`: its `message_id` must not be in the success cooldown
    /// and the exact request must not be in the failure cooldown. Pure check — does **not** record.
    #[must_use]
    pub fn allow(&self, key: &ReobsKey, now: Instant) -> bool {
        self.verified.allow(&key.message_id, now) && self.failed.allow(key, now)
    }

    /// A real `MessagePublished` was re-signed for `message_id`: apply the success cooldown.
    pub fn record_verified(&mut self, message_id: B256, now: Instant) {
        self.verified.record(message_id, now, self.success_interval);
    }

    /// `key` named something we could not verify: apply the short failure cooldown to that exact
    /// request only.
    pub fn record_failed(&mut self, key: ReobsKey, now: Instant) {
        self.failed.record(key, now, self.failure_interval);
    }
}

/// What a processed request earns the [`ReobsRateLimiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settle {
    /// A real `MessagePublished` was re-signed: cool down its `message_id`.
    Verified,
    /// Provably bogus: cool down only this exact request.
    Failed,
    /// No verdict (block not finalized yet, RPC error): no cooldown.
    Unjudged,
}

/// Run one request's `work` under `deadline`, then charge `limiter` from the moment it *finishes*.
/// A request cut off by the deadline is charged as [`Settle::Failed`], so it is not immediately
/// retryable. The worker calls this so the deadline path is accounted for like any other.
pub async fn settle_request(
    limiter: &mut ReobsRateLimiter,
    key: ReobsKey,
    deadline: Duration,
    work: impl std::future::Future<Output = Settle>,
) {
    let settle = tokio::time::timeout(deadline, work)
        .await
        .unwrap_or_else(|_| {
            tracing::warn!(
                "reobservation re-fetch exceeded {deadline:?} — RPC unresponsive; dropping this request"
            );
            Settle::Failed
        });
    let now = Instant::now();
    match settle {
        Settle::Verified => limiter.record_verified(key.message_id, now),
        Settle::Failed => limiter.record_failed(key, now),
        Settle::Unjudged => {}
    }
}

/// Outcome of [`ReobsQueue::push`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueuePush {
    Queued,
    /// An identical request is already queued.
    Duplicate,
    /// The relaying peer already holds [`REOBS_PER_PEER_QUEUE`] slots.
    PeerFull,
    /// The whole queue is at capacity.
    Full,
}

#[derive(Default)]
struct QueueInner {
    items: VecDeque<(libp2p::PeerId, ReobservationRequest)>,
    queued: HashSet<ReobsKey>,
    per_peer: HashMap<libp2p::PeerId, usize>,
}

/// Bounded FIFO of inbound reobservation requests, shared between the p2p task (producer) and the
/// reobservation worker (single consumer). Unlike a plain channel it dedups identical queued requests
/// and caps each relaying peer's share, so replayed or flooded gossip cannot fill every slot and
/// starve a genuine request. A full queue is reported to the caller so it can decline to propagate.
pub struct ReobsQueue {
    inner: Mutex<QueueInner>,
    notify: tokio::sync::Notify,
    capacity: usize,
    per_peer_cap: usize,
}

impl Default for ReobsQueue {
    fn default() -> Self {
        Self::new(REOBS_QUEUE_CAPACITY, REOBS_PER_PEER_QUEUE)
    }
}

impl ReobsQueue {
    #[must_use]
    pub fn new(capacity: usize, per_peer_cap: usize) -> Self {
        Self {
            inner: Mutex::new(QueueInner::default()),
            notify: tokio::sync::Notify::new(),
            capacity: capacity.max(1),
            per_peer_cap: per_peer_cap.max(1),
        }
    }

    /// Non-blocking enqueue on behalf of `peer` (the relaying neighbour).
    pub fn push(&self, peer: libp2p::PeerId, request: ReobservationRequest) -> QueuePush {
        let key = ReobsKey::from(&request);
        let mut inner = self.inner.lock();
        if inner.queued.contains(&key) {
            return QueuePush::Duplicate;
        }
        if inner.items.len() >= self.capacity {
            return QueuePush::Full;
        }
        if inner.per_peer.get(&peer).copied().unwrap_or(0) >= self.per_peer_cap {
            return QueuePush::PeerFull;
        }
        inner.queued.insert(key);
        *inner.per_peer.entry(peer).or_default() += 1;
        inner.items.push_back((peer, request));
        drop(inner);
        // `notify_one` stores a permit when the consumer is not waiting, so a push between its
        // `try_pop` and its `notified().await` is never lost.
        self.notify.notify_one();
        QueuePush::Queued
    }

    fn try_pop(&self) -> Option<ReobservationRequest> {
        let mut inner = self.inner.lock();
        let (peer, request) = inner.items.pop_front()?;
        inner.queued.remove(&ReobsKey::from(&request));
        if let Some(count) = inner.per_peer.get_mut(&peer) {
            *count -= 1;
            if *count == 0 {
                inner.per_peer.remove(&peer);
            }
        }
        Some(request)
    }

    /// Wait for the next request. Single consumer; cancel-safe.
    pub async fn pop(&self) -> ReobservationRequest {
        loop {
            if let Some(request) = self.try_pop() {
                return request;
            }
            self.notify.notified().await;
        }
    }
}

/// Whether the requested block has deterministic source-chain finality. Missing finality never
/// authorizes a vote, regardless of the block's distance from the tip.
fn reobs_final_enough(finalized: Option<u64>, block_height: u64) -> bool {
    finalized.is_some_and(|height| block_height <= height)
}

/// Result of independently re-verifying a [`ReobservationRequest`].
#[derive(Debug)]
pub enum ReobsOutcome {
    /// A genuine, authorized `MessagePublished`: re-sign this.
    Verified(IndexedMessage),
    /// The requested block is above our finalized head. Says nothing about the request's validity
    /// (it may be genuine and just ahead of us), so it must not start any cooldown.
    NotFinalized,
    /// Forged / wrong block / wrong Outbox / `message_id` mismatch. Deliberately not an error: an
    /// unverifiable request is simply ignored.
    Unverifiable,
}

impl ReobsOutcome {
    /// The message to re-sign, if the request verified.
    #[must_use]
    pub fn verified(self) -> Option<IndexedMessage> {
        match self {
            Self::Verified(indexed) => Some(indexed),
            Self::NotFinalized | Self::Unverifiable => None,
        }
    }
}

/// Whether `log` could be the requested emission: `MessagePublished` for the requested `message_id`,
/// from the requested transaction and block, and not removed by a reorg.
fn matches_request(log: &Log, request: &ReobservationRequest) -> bool {
    log.topic0() == Some(&IOutbox::MessagePublished::SIGNATURE_HASH)
        && log.topics().get(1) == Some(&B256::from(request.message_id))
        && !log.removed
        && log.block_number == Some(request.block_height)
        && log.transaction_hash == Some(B256::from(request.tx_hash))
}

/// The logs in `logs` that could be the requested emission, or `None` when more than
/// [`REOBS_MAX_CANDIDATE_LOGS`] qualify (a log-stuffed transaction). The `tx_hash` requirement binds
/// the re-signed vote to the exact emission the requester referenced (audit P3-1).
fn candidate_logs<'a>(logs: &'a [Log], request: &ReobservationRequest) -> Option<Vec<&'a Log>> {
    let mut candidates = Vec::new();
    for log in logs.iter().filter(|log| matches_request(log, request)) {
        if candidates.len() >= REOBS_MAX_CANDIDATE_LOGS {
            return None;
        }
        candidates.push(log);
    }
    Some(candidates)
}

/// Fallback for a block holding more matching logs than the provider will return: read only the
/// *named* transaction's receipt (one call, never every receipt in the block) and keep the logs that
/// could be the emission, stopping one past the cap so a stuffed transaction is detected without
/// copying it all. An absent or mismatched receipt yields no logs.
async fn named_tx_logs<P: Provider>(
    provider: &P,
    request: &ReobservationRequest,
) -> Result<Vec<Log>> {
    let tx_hash = B256::from(request.tx_hash);
    let Some(receipt) = provider
        .get_transaction_receipt(tx_hash)
        .await
        .context("reobservation receipt read failed")?
    else {
        return Ok(Vec::new());
    };
    if receipt.transaction_hash != tx_hash || receipt.block_number != Some(request.block_height) {
        return Ok(Vec::new());
    }
    Ok(receipt
        .inner
        .logs()
        .iter()
        .filter(|log| matches_request(log, request))
        .take(REOBS_MAX_CANDIDATE_LOGS + 1)
        .cloned()
        .collect())
}

/// Re-fetch and re-verify the message against its historical Discovery authorization. `Err` means
/// we could not reach a verdict (RPC failure); it must not start a cooldown either.
///
/// Work is bounded per request: one finalized-head read, one `eth_getLogs` filtered by block, topic
/// and `message_id`, one Discovery lookup, and at most one `isActiveOutbox` per distinct emitter among
/// at most [`REOBS_MAX_CANDIDATE_LOGS`] logs. A random `message_id` costs only the filtered query. The
/// caller bounds the whole call with [`REOBS_RPC_TIMEOUT`].
pub async fn reobserve<P: Provider>(
    provider: &P,
    resolved: &ResolvedRoute,
    _confirmation_depth: u64,
    request: &ReobservationRequest,
) -> Result<ReobsOutcome> {
    let requested_id = B256::from(request.message_id);

    // An unauthenticated pull request must meet the same finality requirement as the listener.
    // Missing/failed reads stop before eth_getLogs; the relayer can retry after RPC recovery.
    // The legacy confirmation-depth argument is retained for caller compatibility only.
    let finalized = provider
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await
        .context("reobservation finalized-head read failed")?
        .context("reobservation finalized head unavailable")?
        .header
        .number;
    let final_enough = reobs_final_enough(Some(finalized), request.block_height);
    if !final_enough {
        tracing::warn!(
            block = request.block_height,
            ?finalized,
            "🔎 reobservation request targets a not-yet-finalized block — ignoring"
        );
        return Ok(ReobsOutcome::NotFinalized);
    }

    // Bound the candidate scan by block and message ID: an id that was never emitted in this block
    // misses the bloom and returns nothing, so it costs one cheap query whatever the block holds.
    let logs = match listener::query_message_logs(
        provider,
        request.block_height,
        request.block_height,
        Some(requested_id),
    )
    .await?
    {
        LogQuery::Logs(logs) => logs,
        // So many matching logs that the provider refuses: only the named transaction can be a
        // genuine emission, so read just its receipt instead of the block's.
        LogQuery::LimitExceeded => named_tx_logs(provider, request).await?,
    };
    let Some(candidates) = candidate_logs(&logs, request) else {
        tracing::warn!(
            block = request.block_height,
            "🔎 reobservation request names a transaction with too many candidate logs — ignoring"
        );
        return Ok(ReobsOutcome::Unverifiable);
    };
    if candidates.is_empty() {
        return Ok(ReobsOutcome::Unverifiable);
    }

    // Authenticate the actual emitting Outbox against the Discovery registry selected at that
    // historical block.
    let Some(discovery) = resolver::discovery_at(provider, resolved, request.block_height).await?
    else {
        return Ok(ReobsOutcome::Unverifiable);
    };
    // One membership lookup per distinct emitter: N logs from one contract cost one `eth_call`.
    let mut memberships: HashMap<Address, bool> = HashMap::new();
    for log in candidates {
        let Ok(decoded) = IOutbox::MessagePublished::decode_log_validate(&log.inner) else {
            continue;
        };
        if decoded.data.messageId != requested_id {
            continue;
        }
        let outbox = log.address();
        let authorized = match memberships.get(&outbox) {
            Some(authorized) => *authorized,
            None => {
                let authorized = resolver::authorized_at(
                    provider,
                    resolved,
                    discovery,
                    outbox,
                    request.block_height,
                )
                .await?;
                memberships.insert(outbox, authorized);
                authorized
            }
        };
        if !authorized {
            continue;
        }
        // `messageId` is already cryptographically bound to emitter/outbox/sequence/payload by
        // `OutboxTypes.computeMessageId` at emission time (asc-contracts #54) — nothing left to
        // recompute; vote on `messageId` itself, same as the listener's fast path.
        return Ok(ReobsOutcome::Verified(IndexedMessage {
            message_id: decoded.data.messageId,
            outbox,
            destination_chain_key: resolved.destination_chain_key,
        }));
    }

    Ok(ReobsOutcome::Unverifiable)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(b: u8) -> B256 {
        B256::from([b; 32])
    }

    #[test]
    fn final_enough_accepts_at_or_below_finalized_head() {
        assert!(reobs_final_enough(Some(100), 100));
        assert!(reobs_final_enough(Some(100), 99));
    }

    #[test]
    fn final_enough_rejects_above_finalized() {
        assert!(!reobs_final_enough(Some(100), 105));
        assert!(!reobs_final_enough(Some(100), 101));
    }

    #[test]
    fn final_enough_rejects_when_no_finalized_tag() {
        assert!(!reobs_final_enough(None, 115));
        assert!(!reobs_final_enough(None, 0));
    }

    #[tokio::test]
    async fn failed_finality_reads_and_unfinalized_requests_never_fetch_logs() {
        use super::super::listener::test_rpc::RpcMock;
        use alloy::providers::ProviderBuilder;
        use serde_json::json;

        let rpc = RpcMock::start().await;
        let provider = ProviderBuilder::new().connect_http(rpc.url.clone());
        let resolved = RpcMock::resolved();
        let request = ReobservationRequest {
            chain_key: 1,
            message_id: [1; 32],
            tx_hash: [2; 32],
            block_height: 105,
        };
        for failure in [
            json!({"result": null}),
            json!({"error": {"code": -32000, "message": "finality unavailable"}}),
        ] {
            rpc.state.lock().finalized_response = failure;
            assert!(reobserve(&provider, &resolved, 3, &request).await.is_err());
        }
        rpc.set_finalized(100);
        assert!(matches!(
            reobserve(&provider, &resolved, 3, &request).await.unwrap(),
            ReobsOutcome::NotFinalized
        ));
        assert!(rpc
            .state
            .lock()
            .requests
            .iter()
            .all(|r| r["method"] == "eth_getBlockByNumber"));

        // Only a recovered finalized head covering the requested block permits a log fetch.
        rpc.set_finalized(105);
        assert!(matches!(
            reobserve(&provider, &resolved, 3, &request).await.unwrap(),
            ReobsOutcome::Unverifiable
        ));
        let scan = rpc.state.lock().requests.last().unwrap().clone();
        assert_eq!(scan["method"], "eth_getLogs");
        assert_eq!(scan["params"][0]["fromBlock"], "0x69");
        assert_eq!(scan["params"][0]["toBlock"], "0x69");
        // A request whose id matches nothing costs one filtered query: no receipt is fetched.
        assert!(rpc
            .state
            .lock()
            .requests
            .iter()
            .all(|r| r["method"] != "eth_getTransactionReceipt"));
    }

    fn key(id_byte: u8, tx_byte: u8, height: u64) -> ReobsKey {
        ReobsKey {
            message_id: id(id_byte),
            tx_hash: id(tx_byte),
            block_height: height,
        }
    }

    #[test]
    fn first_request_is_allowed() {
        let rl = ReobsRateLimiter::new(REOBS_MIN_INTERVAL);
        assert!(rl.allow(&key(1, 1, 10), Instant::now()));
    }

    #[test]
    fn allow_is_pure_and_does_not_record() {
        // Repeated `allow` without a record never denies — the admission check must not itself
        // impose a cooldown.
        let rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        for secs in 0..3 {
            assert!(rl.allow(&key(1, 1, 10), t0 + Duration::from_secs(secs)));
        }
    }

    #[test]
    fn verified_request_uses_full_cooldown_per_message_id() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        rl.record_verified(id(1), t0);
        // Any tuple for the verified message_id is cooling down...
        assert!(!rl.allow(&key(1, 1, 10), t0 + Duration::from_secs(5)));
        assert!(!rl.allow(&key(1, 9, 99), t0 + Duration::from_secs(5)));
        assert!(rl.allow(&key(1, 1, 10), t0 + Duration::from_secs(31)));
        // ...a different message_id is independent.
        assert!(rl.allow(&key(2, 1, 10), t0 + Duration::from_secs(5)));
    }

    #[test]
    fn failed_request_cools_down_only_its_own_tuple() {
        // CSUB-2138: a forged request naming a real message_id with a bogus tx/block must not keep
        // the genuine request (same message_id, different tuple) out.
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(30));
        let t0 = Instant::now();
        let forged = key(1, 9, 999);
        let genuine = key(1, 1, 10);
        rl.record_failed(forged, t0);
        assert!(!rl.allow(
            &forged,
            t0 + REOBS_FAILURE_INTERVAL - Duration::from_millis(1)
        ));
        assert!(rl.allow(&forged, t0 + REOBS_FAILURE_INTERVAL));
        assert!(
            rl.allow(&genuine, t0),
            "the genuine tuple is admissible immediately, however often the forged one repeats"
        );
        // Differing in any single field is a different request.
        assert!(rl.allow(&key(1, 9, 1000), t0));
        assert!(rl.allow(&key(1, 8, 999), t0));
    }

    #[test]
    fn failed_flood_cannot_evict_verified_entries() {
        // Distinct failure keys fill (and churn) only the failure map, so the 30s re-sign bound for
        // a verified message survives a flood of random forged tuples.
        let mut rl = ReobsRateLimiter::with_capacity(Duration::from_secs(30), 2);
        let t0 = Instant::now();
        rl.record_verified(id(1), t0);
        for height in 0..50 {
            rl.record_failed(key(2, 2, height), t0);
        }
        assert!(!rl.allow(&key(1, 1, 10), t0 + Duration::from_secs(5)));
        assert!(rl.failed.next_allowed.len() <= 2);
    }

    #[test]
    fn stale_entries_are_pruned() {
        let mut rl = ReobsRateLimiter::new(Duration::from_secs(10));
        let t0 = Instant::now();
        rl.record_verified(id(1), t0);
        rl.record_failed(key(3, 3, 3), t0);
        // Later records prune entries whose cooldown has elapsed.
        rl.record_verified(id(2), t0 + Duration::from_secs(20));
        rl.record_failed(key(4, 4, 4), t0 + Duration::from_secs(20));
        assert_eq!(rl.verified.next_allowed.len(), 1);
        assert_eq!(rl.failed.next_allowed.len(), 1);
    }

    #[test]
    fn capacity_evicts_oldest_distinct_ids() {
        let mut rl = ReobsRateLimiter::with_capacity(Duration::from_secs(30), 2);
        let now = Instant::now();
        rl.record_verified(id(1), now);
        rl.record_verified(id(2), now);
        rl.record_verified(id(3), now);

        assert!(!rl.verified.next_allowed.contains_key(&id(1)));
        assert!(rl.verified.next_allowed.contains_key(&id(2)));
        assert!(rl.verified.next_allowed.contains_key(&id(3)));
    }

    // ------------------------------* candidate logs *------------------------------ //

    const BLOCK: u64 = 105;

    fn request() -> ReobservationRequest {
        ReobservationRequest {
            chain_key: 1,
            message_id: [1; 32],
            tx_hash: [2; 32],
            block_height: BLOCK,
        }
    }

    fn log(emitter: u8, topic0: B256, msg: u8, tx: u8, block: u64, removed: bool) -> Log {
        serde_json::from_value(serde_json::json!({
            "address": Address::repeat_byte(emitter),
            "topics": [topic0, id(msg)],
            "data": "0x",
            "blockHash": id(9),
            "blockNumber": format!("{block:#x}"),
            "transactionHash": id(tx),
            "transactionIndex": "0x0",
            "logIndex": "0x0",
            "removed": removed,
        }))
        .unwrap()
    }

    fn published(emitter: u8) -> Log {
        log(
            emitter,
            IOutbox::MessagePublished::SIGNATURE_HASH,
            1,
            2,
            BLOCK,
            false,
        )
    }

    #[test]
    fn candidates_are_only_matching_logs_of_the_named_transaction() {
        let sig = IOutbox::MessagePublished::SIGNATURE_HASH;
        let logs = vec![
            log(8, id(7), 1, 2, BLOCK, false),    // another event
            published(3),                         // the candidate
            log(9, sig, 5, 2, BLOCK, false),      // another message id
            log(10, sig, 1, 3, BLOCK, false),     // another transaction
            log(11, sig, 1, 2, BLOCK + 1, false), // another block
            log(12, sig, 1, 2, BLOCK, true),      // reorged away
        ];
        let found = candidate_logs(&logs, &request()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].address(), Address::repeat_byte(3));
    }

    #[test]
    fn log_stuffed_transaction_is_rejected_above_the_cap() {
        let at_cap: Vec<_> = (0..REOBS_MAX_CANDIDATE_LOGS as u8).map(published).collect();
        assert_eq!(
            candidate_logs(&at_cap, &request()).unwrap().len(),
            REOBS_MAX_CANDIDATE_LOGS
        );
        let mut stuffed = at_cap;
        stuffed.push(published(200));
        assert!(candidate_logs(&stuffed, &request()).is_none());
    }

    // ------------------------------* deadline and settling *------------------------------ //

    #[tokio::test]
    async fn a_request_cut_off_by_the_deadline_is_charged_as_failed() {
        let mut rl = ReobsRateLimiter::new(REOBS_MIN_INTERVAL);
        let k = key(1, 2, BLOCK);
        // `work` never finishes, so only the deadline ends it.
        settle_request(
            &mut rl,
            k,
            Duration::from_millis(20),
            std::future::pending::<Settle>(),
        )
        .await;
        let done = Instant::now();
        assert!(
            !rl.allow(&k, done),
            "a timed-out request must not be immediately retryable"
        );
        assert!(
            rl.allow(&k, done + REOBS_FAILURE_INTERVAL),
            "and only the short failure cooldown applies"
        );
        assert!(
            rl.allow(&key(1, 3, BLOCK), done),
            "a different request for the same message_id is unaffected"
        );
    }

    #[tokio::test]
    async fn settled_requests_get_the_cooldown_matching_their_verdict() {
        let mut rl = ReobsRateLimiter::new(REOBS_MIN_INTERVAL);
        let (ok, bad, unjudged) = (key(1, 2, BLOCK), key(3, 2, BLOCK), key(5, 2, BLOCK));
        let deadline = Duration::from_secs(5);
        settle_request(&mut rl, ok, deadline, async { Settle::Verified }).await;
        settle_request(&mut rl, bad, deadline, async { Settle::Failed }).await;
        settle_request(&mut rl, unjudged, deadline, async { Settle::Unjudged }).await;
        let done = Instant::now();
        let later = done + REOBS_FAILURE_INTERVAL + Duration::from_secs(1);
        assert!(
            !rl.allow(&ok, later),
            "verified request keeps the full cooldown"
        );
        assert!(rl.allow(&bad, later), "failed request only the short one");
        assert!(
            rl.allow(&unjudged, done),
            "no verdict means no cooldown at all"
        );
    }

    // ------------------------------* admission queue *------------------------------ //

    fn req(msg: u8) -> ReobservationRequest {
        ReobservationRequest {
            chain_key: 1,
            message_id: [msg; 32],
            tx_hash: [2; 32],
            block_height: BLOCK,
        }
    }

    #[test]
    fn queue_drops_identical_requests() {
        let q = ReobsQueue::new(10, 10);
        let (a, b) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        assert_eq!(q.push(a, req(1)), QueuePush::Queued);
        assert_eq!(q.push(a, req(1)), QueuePush::Duplicate);
        assert_eq!(
            q.push(b, req(1)),
            QueuePush::Duplicate,
            "dedup is origin independent"
        );
        assert_eq!(q.push(a, req(2)), QueuePush::Queued);
    }

    #[test]
    fn one_peer_cannot_fill_the_queue() {
        let q = ReobsQueue::new(10, 3);
        let (a, b) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        for m in 0..3 {
            assert_eq!(q.push(a, req(m)), QueuePush::Queued);
        }
        assert_eq!(q.push(a, req(3)), QueuePush::PeerFull);
        assert_eq!(
            q.push(b, req(4)),
            QueuePush::Queued,
            "another peer still has room"
        );
    }

    #[test]
    fn full_queue_reports_full() {
        let q = ReobsQueue::new(2, 2);
        let (a, b) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        assert_eq!(q.push(a, req(1)), QueuePush::Queued);
        assert_eq!(q.push(b, req(2)), QueuePush::Queued);
        assert_eq!(q.push(libp2p::PeerId::random(), req(3)), QueuePush::Full);
    }

    #[tokio::test]
    async fn pop_is_fifo_and_releases_dedup_and_peer_slots() {
        let q = ReobsQueue::new(10, 1);
        let a = libp2p::PeerId::random();
        assert_eq!(q.push(a, req(1)), QueuePush::Queued);
        assert_eq!(q.push(a, req(2)), QueuePush::PeerFull);
        assert_eq!(q.pop().await, req(1));
        // Both the peer's slot and the dedup entry are free again once popped.
        assert_eq!(q.push(a, req(1)), QueuePush::Queued);
    }

    #[tokio::test]
    async fn pop_waits_for_a_push() {
        let q = std::sync::Arc::new(ReobsQueue::default());
        let waiter = {
            let q = q.clone();
            tokio::spawn(async move { q.pop().await })
        };
        tokio::task::yield_now().await;
        q.push(libp2p::PeerId::random(), req(7));
        let got = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("pop must wake on push")
            .unwrap();
        assert_eq!(got, req(7));
    }
}
