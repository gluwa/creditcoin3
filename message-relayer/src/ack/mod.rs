//! Off-chain acknowledgment submitter (research §05/§10).
//!
//! Trust-minimized acknowledgment is **proof-based, not vote-based**. For each route that opts in
//! (`route.ack = Some(..)`), this worker:
//!
//!  1. Watches the **destination** Inbox for `MessageDelivered(bytes32 indexed messageId)` —
//!     evidence that a message was delivered to the destination dApp.
//!  2. For the transaction that emitted it, fetches a native USC delivery proof from the proof-gen
//!     API (`GET {proof_gen_url}/api/v1/proof-by-tx/{chain_key}/{tx_hash}`): the prover `txBytes`
//!     plus the merkle-inclusion and continuity proofs.
//!  3. Submits that proof to the source-chain `AcknowledgmentValidator.submitAcknowledgment(..)`.
//!     The contract verifies the proof against the block-prover precompile, decodes the
//!     `MessageDelivered` logs, and calls `Outbox.acknowledgeMessage` for each — so the relayer
//!     never needs acknowledge authority; the proof is self-validating.
//!
//! Submission is keyed (and deduped) by destination **transaction hash**: one transaction may
//! contain several `MessageDelivered` logs and the validator acknowledges all of them in a single
//! call. A transaction whose block is not yet attested returns HTTP 422 (`BlockNotReady`) from the
//! proof-gen API and is retried on the next tick.

use std::collections::{HashMap, HashSet, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::network::EthereumWallet;
use alloy::primitives::{Bytes, B256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::rpc::types::Filter;
use alloy::signers::local::PrivateKeySigner;
use alloy::sol_types::{SolError, SolEvent};
use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::abi::{ContinuityProof, IInbox, IOutbox, MerkleProof, MerkleProofEntry};
use crate::checkpoint::CheckpointStore;
use crate::config::{AckConfig, ChainRoute};

/// Poll cadence for the destination `MessageDelivered` watcher and the pending-proof retry queue.
pub const ACK_POLL_INTERVAL_SECS: u64 = 6;

/// Maximum `encodedTransaction` size accepted on-chain by
/// `AcknowledgmentValidator.submitAcknowledgment`. Mirrors `MAX_ENCODED_TRANSACTION_BYTES` in
/// `usc-messaging/contracts/src/AcknowledgmentValidator.sol`; keep the two in sync. Proofs larger
/// than this are rejected on submission (`EncodedTransactionTooLarge`), so we skip them locally
/// instead of paying gas on a guaranteed revert.
pub const MAX_ENCODED_TRANSACTION_BYTES: usize = 500_000;

/// Hard cap on the unacknowledged-tx queue. Without it a prolonged proof-gen outage (or a delivery
/// whose block never attests) would grow `pending` without bound. On overflow the oldest entry is
/// given up (logged) so newer deliveries keep flowing.
const MAX_PENDING_ACKS: usize = 10_000;

/// Hard cap on the set of recently-finished tx hashes kept for in-session dedup. The destination
/// cursor is monotonic, so evicting the oldest entries cannot cause a re-scan to re-process them —
/// this just stops a long-running relayer from leaking one entry per delivery forever.
const MAX_DONE_TRACKED: usize = 10_000;

/// Most pending txs attempted per tick. Bounds how long one tick can run (and how many proof-gen
/// requests it can fan out) regardless of how large `pending` has grown; the rest are retried on
/// subsequent ticks in oldest-first order.
const MAX_ACKS_PER_TICK: usize = 256;

/// Maximum concurrent proof-fetch + submit attempts within a tick. Bounds load on the proof-gen
/// API and the source RPC while still pipelining instead of going strictly serial.
const MAX_ACK_CONCURRENCY: usize = 8;

/// Maximum block span per `eth_getLogs` scan. Public RPCs cap the queryable range; an over-large
/// resume range (long downtime, deep `start_block` backfill) would error on every tick and wedge
/// discovery forever. Bounded chunks advance the cursor incrementally — the 6s tick catches up.
const MAX_BLOCKS_PER_SCAN: u64 = 5_000;

/// Upper bound on waiting for the submitAcknowledgment receipt, so one stuck tx cannot wedge the
/// per-tick pipeline. On timeout the tx stays pending and is retried on a later tick (idempotent:
/// a duplicate lands as `MessageAlreadyAcknowledged`, a terminal revert).
const RECEIPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Retry cadence while the proof block is simply not attested yet (`BlockNotReady`). This is the
/// normal early state of every ack — destination finality plus attestation takes minutes — so it
/// does not count against the transient-failure budget.
const NOT_READY_RETRY: Duration = Duration::from_secs(15);

/// Give up on a tx whose proof never becomes ready (e.g. its block is never attested). Generous:
/// far beyond any healthy finality + attestation latency.
const MAX_ACK_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Exponential backoff for *transient* submit failures (RPC down, timeout, nonce). Doubles from
/// the base per failed attempt, capped, and gives up loudly after [`MAX_ACK_TRANSIENT_ATTEMPTS`] —
/// a permanently failing submit (e.g. unfunded ack signer) must not hammer the RPC every tick
/// forever.
const TRANSIENT_BACKOFF_BASE: Duration = Duration::from_secs(30);
const TRANSIENT_BACKOFF_MAX: Duration = Duration::from_secs(10 * 60);
const MAX_ACK_TRANSIENT_ATTEMPTS: u32 = 20;

/// Spawn the acknowledgment submitter for one route. Returns immediately when the route has no
/// `ack` config; otherwise loops until `cancel` fires or an unrecoverable error occurs.
/// `scan_lookback_blocks` rewinds the persisted cursor on startup so acks that were pending when
/// the process died are re-discovered (see [`crate::config::DEFAULT_SCAN_LOOKBACK_BLOCKS`]).
pub async fn run(
    route: ChainRoute,
    creditcoin_eth_rpc_url: String,
    checkpoint: Option<Arc<CheckpointStore>>,
    scan_lookback_blocks: u64,
    cancel: CancellationToken,
) -> Result<()> {
    let chain_key = route.chain_key;
    let checkpoint_key = format!("ack:{chain_key}");
    let Some(ack) = route.ack.clone() else {
        debug!(chain_key, "ack disabled for route; submitter not started");
        return Ok(());
    };

    // Read-only provider on the destination chain (where MessageDelivered is emitted).
    let dest_provider = ProviderBuilder::new()
        .on_builtin(&route.destination_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: ack submitter failed to connect to destination RPC at {}",
                route.destination_rpc_url
            )
        })?;

    // Wallet-bearing provider on the source (Creditcoin) chain, where we submit the ack.
    let signer: PrivateKeySigner = ack
        .signer_key
        .trim()
        .parse()
        .with_context(|| format!("chain_key {chain_key}: invalid ack.signer_key"))?;
    let submitter_address = signer.address();
    let source_provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .on_builtin(&creditcoin_eth_rpc_url)
        .await
        .with_context(|| {
            format!(
                "chain_key {chain_key}: ack submitter failed to connect to Creditcoin EVM RPC at \
                 {creditcoin_eth_rpc_url}"
            )
        })?;

    let client = ProofGenClient::new(&ack.proof_gen_url)?;

    info!(
        chain_key,
        inbox = %route.inbox_address,
        validator = %ack.validator_address,
        submitter = %submitter_address,
        proof_gen_url = %ack.proof_gen_url,
        "🧾 acknowledgment submitter online"
    );

    // Resume from the persisted cursor so MessageDelivered events emitted while we were down are
    // not skipped; fall back to the current head on first run / when persistence is disabled.
    // The cursor is rewound by `scan_lookback_blocks`: the pending-ack queue is memory-only, so a
    // delivery discovered-but-not-acknowledged before a crash would otherwise be skipped forever.
    // Re-processing is cheap — already-acked / no-ack-needed txs are skipped by the requiresAck
    // pre-check before any proof is fetched.
    let mut last_seen = match checkpoint.as_ref().and_then(|c| c.get(&checkpoint_key)) {
        Some(block) => {
            let resume = block.saturating_sub(scan_lookback_blocks);
            info!(
                chain_key,
                checkpoint = block,
                resume_from = resume + 1,
                "↩️ resuming ack scan from checkpoint (rewound by lookback)"
            );
            resume
        }
        None => {
            if let Some(start) = ack.start_block {
                info!(
                    chain_key,
                    start_block = start,
                    "⏮️ no ack checkpoint; starting initial scan from configured block"
                );
                start.saturating_sub(1)
            } else {
                dest_provider.get_block_number().await.with_context(|| {
                    format!("chain_key {chain_key}: ack submitter failed to read chain head")
                })?
            }
        }
    };

    // Destination tx hashes seen but not yet acknowledged (proof not ready / transient failure).
    let mut pending = PendingAcks::new(MAX_PENDING_ACKS);
    // Tx hashes already acknowledged (or terminally skipped) — never re-submitted (bounded).
    let mut done = BoundedSeen::new(MAX_DONE_TRACKED);

    let mut tick = tokio::time::interval(Duration::from_secs(ACK_POLL_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(chain_key, "🛑 acknowledgment submitter exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                match discover_delivered(
                    chain_key,
                    route.inbox_address,
                    ack.confirmation_depth,
                    &dest_provider,
                    &mut last_seen,
                    &mut pending,
                    &done,
                ).await {
                    Ok(()) => {
                        if let Some(cp) = &checkpoint {
                            if let Err(err) = cp.set(&checkpoint_key, last_seen) {
                                warn!(chain_key, %err, "failed to persist ack checkpoint");
                            }
                        }
                    }
                    Err(err) => warn!(chain_key, %err, "ack discovery iteration failed; will retry"),
                }

                process_pending(
                    chain_key,
                    &ack,
                    route.outbox_address,
                    &client,
                    &source_provider,
                    &mut pending,
                    &mut done,
                ).await;
            }
        }
    }
}

/// Poll the destination Inbox for new `MessageDelivered` events and enqueue their tx hashes.
///
/// Scans only up to `tip - confirmation_depth` so a destination reorg on the unsafe head cannot
/// enqueue an ack for a delivery that later disappears.
async fn discover_delivered<P: Provider>(
    chain_key: u64,
    inbox: alloy::primitives::Address,
    confirmation_depth: u64,
    provider: &P,
    last_seen: &mut u64,
    pending: &mut PendingAcks,
    done: &BoundedSeen,
) -> Result<()> {
    let tip = provider.get_block_number().await?;
    let confirmed = tip.saturating_sub(confirmation_depth);
    if confirmed <= *last_seen {
        return Ok(());
    }
    let from_block = *last_seen + 1;
    // Bounded chunk (see MAX_BLOCKS_PER_SCAN): never ask the RPC for more than it will serve.
    let to_block = confirmed.min(last_seen.saturating_add(MAX_BLOCKS_PER_SCAN));

    let filter = Filter::new()
        .address(inbox)
        .event_signature(IInbox::MessageDelivered::SIGNATURE_HASH)
        .from_block(from_block)
        .to_block(to_block);

    let logs = provider.get_logs(&filter).await.with_context(|| {
        format!("eth_getLogs MessageDelivered from {from_block} to {to_block} failed")
    })?;

    for log in logs {
        let Some(tx_hash) = log.transaction_hash else {
            warn!(
                chain_key,
                "MessageDelivered log without transaction_hash; skipping"
            );
            continue;
        };
        if log.block_number.is_none() {
            warn!(
                chain_key,
                %tx_hash,
                "MessageDelivered log without block_number; skipping"
            );
            continue;
        }
        // The delivered messageId feeds the requiresAck pre-check on the source Outbox, so bridge
        // traffic never costs a proof fetch. A tx may carry several MessageDelivered logs.
        let message_id = match IInbox::MessageDelivered::decode_log(&log.inner, true) {
            Ok(decoded) => decoded.data.messageId,
            Err(err) => {
                warn!(chain_key, %tx_hash, %err, "could not decode MessageDelivered log; skipping");
                continue;
            }
        };
        if done.contains(&tx_hash) {
            continue;
        }
        if pending.contains(&tx_hash) {
            pending.note_message_id(&tx_hash, message_id);
            continue;
        }
        if let Some(evicted) = pending.insert(tx_hash, Instant::now(), vec![message_id]) {
            warn!(
                chain_key,
                %evicted,
                "ack pending queue at capacity; giving up on oldest un-acknowledged delivery"
            );
        }
        debug!(chain_key, %tx_hash, %message_id, "🧾 observed MessageDelivered; queued for acknowledgment");
    }

    *last_seen = to_block;
    Ok(())
}

/// Try to fetch a proof and submit an acknowledgment for every pending destination tx that is due
/// for an attempt. Successful (or terminally-reverting) submissions move to `done`; not-yet-ready
/// proofs are deferred by [`NOT_READY_RETRY`]; transient failures back off exponentially and give
/// up after [`MAX_ACK_TRANSIENT_ATTEMPTS`].
#[allow(clippy::too_many_arguments)]
async fn process_pending<P: Provider>(
    chain_key: u64,
    ack: &AckConfig,
    outbox: Option<alloy::primitives::Address>,
    client: &ProofGenClient,
    source_provider: &P,
    pending: &mut PendingAcks,
    done: &mut BoundedSeen,
) {
    // Retry oldest-first, a bounded batch per tick, so a large backlog cannot make one tick run
    // unboundedly long (or starve `discover_delivered` / shutdown).
    let now = Instant::now();
    let batch = pending.ready(MAX_ACKS_PER_TICK, now);
    if batch.is_empty() {
        return;
    }

    // Fetch proofs + submit with bounded concurrency rather than strictly serially: each attempt
    // is independent and dominated by network latency. Mutations to `pending`/`done` are applied
    // afterwards, on this task, so no shared-state synchronization is needed.
    let results: Vec<(B256, Result<AckOutcome>)> = futures::stream::iter(batch)
        .map(|(tx_hash, message_ids)| async move {
            let outcome = acknowledge_tx(
                chain_key,
                ack,
                outbox,
                &message_ids,
                client,
                source_provider,
                tx_hash,
            )
            .await;
            (tx_hash, outcome)
        })
        .buffer_unordered(MAX_ACK_CONCURRENCY)
        .collect()
        .await;

    let now = Instant::now();
    for (tx_hash, outcome) in results {
        match outcome {
            Ok(AckOutcome::Acknowledged) => {
                info!(chain_key, %tx_hash, "✅ delivery acknowledged on source Outbox");
                pending.remove(&tx_hash);
                done.insert(tx_hash);
            }
            Ok(AckOutcome::Terminal(reason)) => {
                info!(chain_key, %tx_hash, %reason, "ack skipped (terminal); will not retry");
                pending.remove(&tx_hash);
                done.insert(tx_hash);
            }
            Ok(AckOutcome::NotReady) => {
                // Normal early state (destination finality + attestation take minutes) — defer
                // without burning the transient budget, but don't wait forever on a block that
                // never attests.
                if pending.age(&tx_hash, now) > Some(MAX_ACK_AGE) {
                    warn!(
                        chain_key,
                        %tx_hash,
                        "proof never became ready within {MAX_ACK_AGE:?}; giving up on this ack"
                    );
                    pending.remove(&tx_hash);
                    done.insert(tx_hash);
                } else {
                    debug!(chain_key, %tx_hash, "proof not ready yet; deferred");
                    pending.defer(&tx_hash, now + NOT_READY_RETRY);
                }
            }
            Err(err) => {
                let attempts = pending.record_transient_failure(&tx_hash, now);
                if attempts >= MAX_ACK_TRANSIENT_ATTEMPTS {
                    warn!(
                        chain_key,
                        %tx_hash,
                        attempts,
                        %err,
                        "ack transient-failure budget exhausted; giving up (operator action likely \
                         required — check the ack signer balance and the Creditcoin RPC)"
                    );
                    pending.remove(&tx_hash);
                    done.insert(tx_hash);
                } else {
                    warn!(
                        chain_key,
                        %tx_hash,
                        attempts,
                        %err,
                        "ack attempt failed transiently; will retry with backoff"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Bounded tracking structures
// ---------------------------------------------------------------------------

/// One destination tx awaiting acknowledgment.
struct PendingAck {
    /// When the delivery was first observed — drives cap eviction and the [`MAX_ACK_AGE`] cutoff.
    first_seen: Instant,
    /// Earliest next attempt ([`NOT_READY_RETRY`] deferral or transient backoff).
    next_attempt_at: Instant,
    /// Transient submit failures so far (NotReady deferrals do not count).
    transient_attempts: u32,
    /// messageIds decoded from this tx's `MessageDelivered` logs, for the requiresAck pre-check.
    message_ids: Vec<B256>,
}

/// Destination tx hashes awaiting acknowledgment, bounded by [`MAX_PENDING_ACKS`]. Retried
/// oldest-first; on overflow the oldest entry is evicted (an unacknowledged delivery we give up
/// on) so the queue cannot grow without limit during a proof-gen outage.
struct PendingAcks {
    seen: HashMap<B256, PendingAck>,
    cap: usize,
}

impl PendingAcks {
    fn new(cap: usize) -> Self {
        Self {
            seen: HashMap::new(),
            cap,
        }
    }

    fn contains(&self, tx: &B256) -> bool {
        self.seen.contains_key(tx)
    }

    fn remove(&mut self, tx: &B256) {
        self.seen.remove(tx);
    }

    /// How long `tx` has been tracked, or `None` if unknown.
    fn age(&self, tx: &B256, now: Instant) -> Option<Duration> {
        self.seen
            .get(tx)
            .map(|e| now.saturating_duration_since(e.first_seen))
    }

    /// Track a newly-observed tx (no-op if already tracked). Returns the tx hash evicted to honour
    /// the cap, if any — the caller logs it as an unacknowledged delivery being abandoned.
    fn insert(&mut self, tx: B256, now: Instant, message_ids: Vec<B256>) -> Option<B256> {
        if self.seen.contains_key(&tx) {
            return None;
        }
        self.seen.insert(
            tx,
            PendingAck {
                first_seen: now,
                next_attempt_at: now,
                transient_attempts: 0,
                message_ids,
            },
        );
        if self.seen.len() > self.cap {
            if let Some((&oldest, _)) = self.seen.iter().min_by_key(|(_, e)| e.first_seen) {
                self.seen.remove(&oldest);
                return Some(oldest);
            }
        }
        None
    }

    /// Append another delivered messageId to an already-tracked tx (a tx may emit several
    /// `MessageDelivered` logs discovered across scans).
    fn note_message_id(&mut self, tx: &B256, message_id: B256) {
        if let Some(entry) = self.seen.get_mut(tx) {
            if !entry.message_ids.contains(&message_id) {
                entry.message_ids.push(message_id);
            }
        }
    }

    /// The oldest `n` tx hashes whose next attempt is due, oldest-first, with their messageIds.
    fn ready(&self, n: usize, now: Instant) -> Vec<(B256, Vec<B256>)> {
        let mut entries: Vec<(B256, Instant, Vec<B256>)> = self
            .seen
            .iter()
            .filter(|(_, e)| e.next_attempt_at <= now)
            .map(|(&h, e)| (h, e.first_seen, e.message_ids.clone()))
            .collect();
        entries.sort_by_key(|&(_, first_seen, _)| first_seen);
        entries
            .into_iter()
            .take(n)
            .map(|(h, _, ids)| (h, ids))
            .collect()
    }

    /// Defer the next attempt for `tx` (proof not ready — not a failure).
    fn defer(&mut self, tx: &B256, until: Instant) {
        if let Some(entry) = self.seen.get_mut(tx) {
            entry.next_attempt_at = until;
        }
    }

    /// Record a transient submit failure: bump the attempt counter and schedule the next try with
    /// exponential backoff. Returns the attempts so far so the caller can enforce the budget.
    fn record_transient_failure(&mut self, tx: &B256, now: Instant) -> u32 {
        let Some(entry) = self.seen.get_mut(tx) else {
            return 0;
        };
        entry.transient_attempts += 1;
        let exp = entry.transient_attempts.saturating_sub(1).min(31);
        let backoff = TRANSIENT_BACKOFF_BASE
            .saturating_mul(2u32.saturating_pow(exp))
            .min(TRANSIENT_BACKOFF_MAX);
        entry.next_attempt_at = now + backoff;
        entry.transient_attempts
    }
}

/// A FIFO set of fixed capacity: insertion past `cap` evicts the oldest entry. Used to remember
/// recently-finished tx hashes for in-session dedup without leaking memory over long uptimes.
struct BoundedSeen {
    set: HashSet<B256>,
    order: VecDeque<B256>,
    cap: usize,
}

impl BoundedSeen {
    fn new(cap: usize) -> Self {
        Self {
            set: HashSet::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn contains(&self, tx: &B256) -> bool {
        self.set.contains(tx)
    }

    fn insert(&mut self, tx: B256) {
        if self.set.insert(tx) {
            self.order.push_back(tx);
            if self.set.len() > self.cap {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
}

enum AckOutcome {
    /// Proof verified and `acknowledgeMessage` succeeded.
    Acknowledged,
    /// The proof block is not yet attested (`BlockNotReady`); retry later.
    NotReady,
    /// A permanent condition (e.g. on-chain revert: already acknowledged / does not require ack).
    Terminal(String),
}

/// Fetch the delivery proof for `tx_hash` and submit it to the source `AcknowledgmentValidator`.
///
/// Before any proof is fetched, the delivered messageIds are checked against the source Outbox:
/// when none requires an acknowledgment (bridge traffic publishes `requiresAck = false`) or all
/// are already acknowledged, the tx is terminal without costing a proof-gen round-trip or a
/// guaranteed-revert gas estimate.
#[allow(clippy::too_many_arguments)]
async fn acknowledge_tx<P: Provider>(
    chain_key: u64,
    ack: &AckConfig,
    outbox: Option<alloy::primitives::Address>,
    message_ids: &[B256],
    client: &ProofGenClient,
    source_provider: &P,
    tx_hash: B256,
) -> Result<AckOutcome> {
    // Fail open: if the view calls error we fall through to the proof path — the validator
    // enforces the same rules on-chain, this is purely a cost-saving shortcut.
    if let Some(outbox_addr) = outbox {
        if !message_ids.is_empty() {
            match any_requires_ack(source_provider, outbox_addr, message_ids).await {
                Ok(false) => {
                    return Ok(AckOutcome::Terminal(
                        "no delivered message requires acknowledgment (requiresAck=false or \
                         already acknowledged)"
                            .into(),
                    ));
                }
                Ok(true) => {}
                Err(err) => {
                    warn!(chain_key, %tx_hash, %err, "requiresAck pre-check failed; proceeding with proof");
                }
            }
        }
    }

    let proof = match client.proof_by_tx(chain_key, tx_hash).await? {
        ProofFetch::Ready(p) => p,
        ProofFetch::NotReady => return Ok(AckOutcome::NotReady),
    };

    let encoded_tx = proof.encoded_transaction()?;

    // Mirror the on-chain cap: an oversized encodedTransaction is rejected by submitAcknowledgment
    // (EncodedTransactionTooLarge), so skip it before spending gas on a guaranteed revert. This is a
    // permanent condition for this proof, hence Terminal (no retry).
    if encoded_tx.len() > MAX_ENCODED_TRANSACTION_BYTES {
        return Ok(AckOutcome::Terminal(format!(
            "encodedTransaction {} bytes exceeds on-chain max {} bytes",
            encoded_tx.len(),
            MAX_ENCODED_TRANSACTION_BYTES
        )));
    }

    let (merkle_proof, continuity_proof) = proof.to_proofs()?;
    let height = proof.header_number;

    let validator =
        crate::abi::IAcknowledgmentValidator::new(ack.validator_address, source_provider);

    let pending_tx = validator
        .submitAcknowledgment(height, encoded_tx, merkle_proof, continuity_proof)
        .send()
        .await;

    match pending_tx {
        Ok(builder) => match tokio::time::timeout(RECEIPT_TIMEOUT, builder.get_receipt()).await {
            Err(_elapsed) => Err(anyhow!(
                "no receipt within {RECEIPT_TIMEOUT:?} (ack tx possibly stuck)"
            )),
            Ok(receipt_result) => match receipt_result {
                Ok(receipt) if receipt.status() => {
                    // Report the on-chain gas cost of the submitAcknowledgment call.
                    // gas_used is cast to u128 so the arithmetic is valid regardless of
                    // the receipt's integer width; the wide-integer fields are recorded
                    // via Display (`%`) because `tracing` has no native u128 Value impl.
                    let gas_used = u128::from(receipt.gas_used);
                    let effective_gas_price = receipt.effective_gas_price;
                    let gas_cost_wei = gas_used.saturating_mul(effective_gas_price);
                    info!(
                        chain_key,
                        %tx_hash,
                        ack_tx_hash = %receipt.transaction_hash,
                        gas_used = %gas_used,
                        effective_gas_price_wei = %effective_gas_price,
                        gas_cost_wei = %gas_cost_wei,
                        "submitAcknowledgment confirmed",
                    );
                    Ok(AckOutcome::Acknowledged)
                }
                Ok(_) => Ok(AckOutcome::Terminal("tx mined but reverted".into())),
                Err(err) => Err(anyhow!("receipt fetch failed: {err}")),
            },
        },
        Err(err) if is_terminal_revert(&err) => Ok(AckOutcome::Terminal(describe_revert(&err))),
        Err(err) => Err(anyhow!("submitAcknowledgment send failed: {err}")),
    }
}

/// Whether any of `ids` still needs an acknowledgment on the source Outbox: published with
/// `requiresAck = true`, known to the outbox (`emitter != 0`), and not yet acknowledged.
async fn any_requires_ack<P: Provider>(
    provider: &P,
    outbox: alloy::primitives::Address,
    ids: &[B256],
) -> Result<bool> {
    let outbox = IOutbox::new(outbox, provider);
    for id in ids {
        if !outbox
            .messageRequiresAck(*id)
            .call()
            .await
            .context("IOutbox.messageRequiresAck call failed")?
            ._0
        {
            continue;
        }
        let m = outbox
            .messages(*id)
            .call()
            .await
            .context("IOutbox.messages call failed")?;
        if m.emitter != alloy::primitives::Address::ZERO && !m.acknowledged {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Decode a known ack-path revert selector into its error name, for actionable Terminal logs
/// (Creditcoin's EVM RPC returns raw selector data with no decoded name). Selectors come from the
/// shared [`write_ability::abi`] `sol!` declarations, so they cannot drift from the contracts.
fn ack_revert_name(sel: [u8; 4]) -> Option<&'static str> {
    use crate::abi::{IAcknowledgmentValidator as V, IOutbox as O};
    Some(match sel {
        s if s == O::MessageDoesNotRequireAck::SELECTOR => "MessageDoesNotRequireAck",
        s if s == O::MessageAlreadyAcknowledged::SELECTOR => "MessageAlreadyAcknowledged",
        s if s == O::MessageNotFound::SELECTOR => "MessageNotFound",
        s if s == V::ProofVerificationFailed::SELECTOR => "ProofVerificationFailed",
        s if s == V::NoMessageDeliveredLogs::SELECTOR => "NoMessageDeliveredLogs",
        _ => return None,
    })
}

/// Classify a submit error as a permanent on-chain revert (vs. a transient RPC failure). A contract
/// revert observed at send / gas-estimation time is deterministic, so it must be terminal — otherwise
/// the tx is retried forever (e.g. every bridge Release delivery, which reverts
/// `MessageDoesNotRequireAck`). Matches revert phrasing across node dialects (see [`crate::revert`])
/// plus decoded error names as a fallback.
fn is_terminal_revert(err: &impl std::fmt::Display) -> bool {
    let s = err.to_string();
    if crate::revert::is_revert(&s) {
        return true;
    }
    // Decoded error-name fallback (nodes that surface the custom-error name without standard
    // revert phrasing).
    s.contains("AlreadyAcknowledged")
        || s.contains("DoesNotRequireAck")
        || s.contains("MessageNotFound")
        || s.contains("ProofVerificationFailed")
        || s.contains("NoMessageDeliveredLogs")
}

/// Prefix a terminal revert with the decoded error name when the selector is recognized, so the
/// `ack skipped (terminal)` log tells the operator *why* without a manual selector lookup.
fn describe_revert(err: &impl std::fmt::Display) -> String {
    let s = err.to_string();
    match crate::revert::revert_selector(&s).and_then(ack_revert_name) {
        Some(name) => format!("{name}: {s}"),
        None => s,
    }
}

// ---------------------------------------------------------------------------
// proof-gen API client
// ---------------------------------------------------------------------------

/// Minimal HTTP client for the proof-gen API server's `proof-by-tx` endpoint.
struct ProofGenClient {
    http: reqwest::Client,
    base: String,
}

enum ProofFetch {
    Ready(SingleContinuityResponse),
    /// HTTP 422 — the block containing the tx is not yet attested.
    NotReady,
}

impl ProofGenClient {
    fn new(base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build proof-gen HTTP client")?;
        Ok(Self {
            http,
            base: base_url.trim_end_matches('/').to_string(),
        })
    }

    async fn proof_by_tx(&self, chain_key: u64, tx_hash: B256) -> Result<ProofFetch> {
        let url = format!(
            "{}/api/v1/proof-by-tx/{}/{:#x}",
            self.base, chain_key, tx_hash
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url} failed"))?;

        // 422 (BlockNotReady) is expected while the destination block is still being attested.
        if resp.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return Ok(ProofFetch::NotReady);
        }
        let status = resp.status();
        let body = resp
            .text()
            .await
            .with_context(|| format!("reading body of {url}"))?;
        if !status.is_success() {
            anyhow::bail!("proof-gen returned {status} for {url}: {body}");
        }
        let parsed: SingleContinuityResponse = serde_json::from_str(&body)
            .with_context(|| format!("decoding proof-gen response from {url}"))?;
        Ok(ProofFetch::Ready(parsed))
    }
}

// ---------------------------------------------------------------------------
// proof-gen response shape (mirrors proof-gen-api-server SingleContinuityResponse)
// ---------------------------------------------------------------------------

/// Subset of the proof-gen `SingleContinuityResponse` the submitter needs. Field names are
/// camelCase to match the server's `#[serde(rename_all = "camelCase")]`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SingleContinuityResponse {
    header_number: u64,
    /// Hex-encoded prover `txBytes` (encoded tx + receipt). `None` when the server only returned a
    /// continuity proof (no merkle inclusion) — which would not satisfy the validator.
    tx_bytes: Option<String>,
    continuity_proof: ContinuityProofJson,
    merkle_proof: MerkleProofJson,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContinuityProofJson {
    lower_endpoint_digest: String,
    roots: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MerkleProofJson {
    root: String,
    siblings: Vec<MerkleProofEntryJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MerkleProofEntryJson {
    hash: String,
    is_left: bool,
}

impl SingleContinuityResponse {
    /// Hex-decode the prover `txBytes` into the calldata the validator expects.
    fn encoded_transaction(&self) -> Result<Bytes> {
        let raw = self.tx_bytes.as_deref().context(
            "proof-gen response missing txBytes (continuity-only proof cannot be acked)",
        )?;
        let bytes =
            hex::decode(raw.trim_start_matches("0x")).context("txBytes is not valid hex")?;
        Ok(Bytes::from(bytes))
    }

    /// Convert the JSON proof bundle into the `sol!`-generated argument structs.
    fn to_proofs(&self) -> Result<(MerkleProof, ContinuityProof)> {
        let merkle = MerkleProof {
            root: parse_b256(&self.merkle_proof.root).context("merkle_proof.root")?,
            siblings: self
                .merkle_proof
                .siblings
                .iter()
                .map(|s| {
                    Ok(MerkleProofEntry {
                        hash: parse_b256(&s.hash).context("merkle_proof.siblings[].hash")?,
                        isLeft: s.is_left,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };

        let continuity = ContinuityProof {
            lowerEndpointDigest: parse_b256(&self.continuity_proof.lower_endpoint_digest)
                .context("continuity_proof.lower_endpoint_digest")?,
            roots: self
                .continuity_proof
                .roots
                .iter()
                .map(|r| parse_b256(r).context("continuity_proof.roots[]"))
                .collect::<Result<Vec<_>>>()?,
        };

        Ok((merkle, continuity))
    }
}

fn parse_b256(s: &str) -> Result<B256> {
    B256::from_str(s.trim()).with_context(|| format!("not a 32-byte hex value: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "chainKey": 2,
        "headerNumber": 123,
        "txIndex": 0,
        "txHash": "0x1111111111111111111111111111111111111111111111111111111111111111",
        "txBytes": "0xdeadbeef",
        "continuityProof": {
            "lowerEndpointDigest": "0x2222222222222222222222222222222222222222222222222222222222222222",
            "roots": [
                "0x3333333333333333333333333333333333333333333333333333333333333333",
                "0x4444444444444444444444444444444444444444444444444444444444444444"
            ]
        },
        "merkleProof": {
            "root": "0x5555555555555555555555555555555555555555555555555555555555555555",
            "siblings": [
                { "hash": "0x6666666666666666666666666666666666666666666666666666666666666666", "isLeft": true },
                { "hash": "0x7777777777777777777777777777777777777777777777777777777777777777", "isLeft": false }
            ]
        },
        "cached": false,
        "generatedAt": "2026-06-24T00:00:00Z"
    }"#;

    #[test]
    fn decodes_proof_gen_response() {
        let parsed: SingleContinuityResponse = serde_json::from_str(SAMPLE).unwrap();
        assert_eq!(parsed.header_number, 123);

        let enc = parsed.encoded_transaction().unwrap();
        assert_eq!(enc.to_vec(), vec![0xde, 0xad, 0xbe, 0xef]);

        let (merkle, continuity) = parsed.to_proofs().unwrap();
        assert_eq!(merkle.siblings.len(), 2);
        assert!(merkle.siblings[0].isLeft);
        assert!(!merkle.siblings[1].isLeft);
        assert_eq!(continuity.roots.len(), 2);
    }

    #[test]
    fn missing_tx_bytes_is_error() {
        let json = SAMPLE.replace("\"txBytes\": \"0xdeadbeef\",", "");
        let parsed: SingleContinuityResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.encoded_transaction().is_err());
    }

    #[test]
    fn terminal_revert_classification() {
        // Decoded-name form (nodes that surface the custom-error name).
        assert!(is_terminal_revert(&"reverted: MessageAlreadyAcknowledged"));
        assert!(is_terminal_revert(&"execution reverted"));

        // Creditcoin EVM RPC form: raw selector data, no decoded name — this is what was slipping
        // through and retrying forever on every bridge Release delivery (MessageDoesNotRequireAck).
        assert!(is_terminal_revert(
            &"submitAcknowledgment send failed: server returned an error response: error code \
              -32603: VM Exception while processing transaction: revert, data: \
              \"0x2f28bb55c8e0b2db4217508f44fb2d148bd9fab3c94e876a56a3fdbcf71f17570ecbe54c\""
        ));
        // Unknown selector but a clear revert phrasing is still terminal.
        assert!(is_terminal_revert(
            &"VM Exception while processing transaction: revert, data: \"0xdeadbeef\""
        ));

        // Genuine transient infra failures stay retryable.
        assert!(!is_terminal_revert(
            &"error sending request: connection refused"
        ));
        assert!(!is_terminal_revert(
            &"server returned an error response: error code -32000: insufficient funds for gas"
        ));
    }

    #[test]
    fn describe_revert_decodes_known_selectors() {
        // The real-world Creditcoin node string for a bridge Release delivery: raw selector, no
        // name. The Terminal log should still name the error for the operator.
        let s = "VM Exception while processing transaction: revert, data: \
                 \"0x2f28bb55c8e0b2db4217508f44fb2d148bd9fab3c94e876a56a3fdbcf71f17570ecbe54c\"";
        assert!(describe_revert(&s).starts_with("MessageDoesNotRequireAck: "));
        // Unknown selector passes through unchanged.
        let unknown = "revert, data: \"0xdeadbeef\"";
        assert_eq!(describe_revert(&unknown), unknown);
        // Selector constants in the shared ABI must still hash to the on-wire values.
        assert_eq!(
            crate::abi::IOutbox::MessageDoesNotRequireAck::SELECTOR,
            [0x2f, 0x28, 0xbb, 0x55]
        );
        assert_eq!(
            crate::abi::IOutbox::MessageAlreadyAcknowledged::SELECTOR,
            [0x33, 0x70, 0x4b, 0x28]
        );
        assert_eq!(
            crate::abi::IOutbox::MessageNotFound::SELECTOR,
            [0x5d, 0x80, 0x3c, 0xca]
        );
    }

    fn tx(n: u8) -> B256 {
        B256::from([n; 32])
    }

    /// `ready(..)` tx hashes only, for FIFO assertions.
    fn ready_txs(p: &PendingAcks, n: usize, now: Instant) -> Vec<B256> {
        p.ready(n, now).into_iter().map(|(h, _)| h).collect()
    }

    #[test]
    fn pending_dedupes_and_reports_no_eviction_under_cap() {
        let mut p = PendingAcks::new(4);
        let now = Instant::now();
        assert!(p.insert(tx(1), now, vec![tx(9)]).is_none());
        assert!(p.contains(&tx(1)));
        // Re-inserting the same tx is a no-op (no eviction, still tracked once).
        assert!(p.insert(tx(1), now, vec![tx(9)]).is_none());
        assert_eq!(ready_txs(&p, 10, now), vec![tx(1)]);
        // The messageIds ride along for the requiresAck pre-check.
        assert_eq!(p.ready(10, now)[0].1, vec![tx(9)]);
        // note_message_id appends without duplicating.
        p.note_message_id(&tx(1), tx(9));
        p.note_message_id(&tx(1), tx(8));
        assert_eq!(p.ready(10, now)[0].1, vec![tx(9), tx(8)]);
    }

    #[test]
    fn pending_evicts_oldest_on_overflow() {
        let mut p = PendingAcks::new(2);
        let t0 = Instant::now();
        // Distinct, increasing timestamps so "oldest" is unambiguous.
        assert!(p.insert(tx(1), t0, Vec::new()).is_none());
        assert!(p
            .insert(tx(2), t0 + Duration::from_millis(1), Vec::new())
            .is_none());
        // Third insert exceeds cap → evicts tx(1), the oldest.
        let evicted = p.insert(tx(3), t0 + Duration::from_millis(2), Vec::new());
        assert_eq!(evicted, Some(tx(1)));
        assert!(!p.contains(&tx(1)));
        assert!(p.contains(&tx(2)));
        assert!(p.contains(&tx(3)));
    }

    #[test]
    fn pending_ready_is_fifo_and_bounded_by_n() {
        let mut p = PendingAcks::new(100);
        let t0 = Instant::now();
        for i in 0..5u8 {
            p.insert(tx(i), t0 + Duration::from_millis(u64::from(i)), Vec::new());
        }
        let now = t0 + Duration::from_secs(1);
        assert_eq!(ready_txs(&p, 3, now), vec![tx(0), tx(1), tx(2)]);
        p.remove(&tx(0));
        assert_eq!(ready_txs(&p, 3, now), vec![tx(1), tx(2), tx(3)]);
    }

    #[test]
    fn pending_defer_and_backoff_gate_readiness() {
        let mut p = PendingAcks::new(10);
        let t0 = Instant::now();
        p.insert(tx(1), t0, Vec::new());
        assert_eq!(ready_txs(&p, 10, t0), vec![tx(1)]);

        // NotReady deferral: hidden until `until`, no attempt counted.
        p.defer(&tx(1), t0 + NOT_READY_RETRY);
        assert!(ready_txs(&p, 10, t0).is_empty());
        assert_eq!(ready_txs(&p, 10, t0 + NOT_READY_RETRY), vec![tx(1)]);

        // Transient failures back off exponentially: 30s, 60s, 120s, … capped at 10min.
        assert_eq!(p.record_transient_failure(&tx(1), t0), 1);
        assert!(ready_txs(&p, 10, t0 + TRANSIENT_BACKOFF_BASE - Duration::from_secs(1)).is_empty());
        assert_eq!(ready_txs(&p, 10, t0 + TRANSIENT_BACKOFF_BASE), vec![tx(1)]);
        assert_eq!(p.record_transient_failure(&tx(1), t0), 2);
        assert!(ready_txs(&p, 10, t0 + TRANSIENT_BACKOFF_BASE).is_empty());
        assert_eq!(
            ready_txs(&p, 10, t0 + TRANSIENT_BACKOFF_BASE * 2),
            vec![tx(1)]
        );
        // Deep attempt counts saturate at the cap instead of overflowing.
        for _ in 0..40 {
            p.record_transient_failure(&tx(1), t0);
        }
        assert_eq!(ready_txs(&p, 10, t0 + TRANSIENT_BACKOFF_MAX), vec![tx(1)]);

        // Age is measured from first_seen, unaffected by deferrals.
        assert_eq!(
            p.age(&tx(1), t0 + Duration::from_secs(5)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(p.age(&tx(2), t0), None);
    }

    #[test]
    fn bounded_seen_is_fifo_capped() {
        let mut s = BoundedSeen::new(2);
        s.insert(tx(1));
        s.insert(tx(1)); // idempotent
        s.insert(tx(2));
        s.insert(tx(3)); // evicts tx(1)
        assert!(!s.contains(&tx(1)));
        assert!(s.contains(&tx(2)));
        assert!(s.contains(&tx(3)));
    }
}
