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
//! The worst a forged or spammed request can do is make us perform a bounded `eth_getLogs` — bounded
//! because [`ReobsRateLimiter`] drops repeat requests inside a cooldown. Only a *verified* request
//! cools down its `message_id`; a failed one cools down just its own `(message_id, tx_hash,
//! block_height)`, so a forged request can never lock out the genuine one (CSUB-2138).
//!
//! [`listener`]: super::listener
//! [`MessageVote`]: write_ability::envelope::MessageVote
//! [`ReobservationRequest`]: write_ability::envelope::ReobservationRequest

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::time::{Duration, Instant};

use alloy::primitives::B256;
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

use write_ability::abi::IOutbox;
use write_ability::envelope::ReobservationRequest;

use super::listener::IndexedMessage;
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

/// Per-request wall-clock deadline for the tip + `eth_getLogs` re-fetch. A black-holed RPC must not
/// let a single reobservation hang the responder; on timeout the request is dropped (the relayer
/// re-requests on its own cadence). Matches the listener's `RPC_TIMEOUT`.
pub const REOBS_RPC_TIMEOUT: Duration = Duration::from_secs(30);

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

/// Re-fetch and re-verify the message against its historical Discovery authorization. `Err` means
/// we could not reach a verdict (RPC failure); it must not start a cooldown either.
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

    // Bound the candidate scan by block and message ID, then authenticate the actual emitting
    // Outbox against the Discovery registry selected at that historical block.
    let logs = super::listener::fetch_message_logs(
        provider,
        request.block_height,
        request.block_height,
        Some(requested_id),
    )
    .await?;

    if logs.is_empty() {
        return Ok(ReobsOutcome::Unverifiable);
    }
    let Some(discovery) = resolver::discovery_at(provider, resolved, request.block_height).await?
    else {
        return Ok(ReobsOutcome::Unverifiable);
    };
    let requested_tx = B256::from(request.tx_hash);
    for log in logs {
        // Verify the log came from the transaction the request named (audit P3-1). The `tx_hash`
        // field previously rode along unchecked; enforce it so the re-signed vote is bound to the
        // exact emission the requester referenced. Well-defined because the finality gate above only
        // let us past only for a GRANDPA-finalized block, so the transaction hash is stable. (`message_id` + Outbox +
        // block already pin the message; this closes the "field implies a correlation the code
        // doesn't perform" gap.)
        if log.removed
            || log.block_number != Some(request.block_height)
            || log.transaction_hash != Some(requested_tx)
        {
            continue;
        }
        let Ok(decoded) = IOutbox::MessagePublished::decode_log_validate(&log.inner) else {
            continue;
        };
        if decoded.data.messageId != requested_id {
            continue;
        }
        let outbox = log.address();
        if !resolver::authorized_at(provider, resolved, discovery, outbox, request.block_height)
            .await?
        {
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
}
