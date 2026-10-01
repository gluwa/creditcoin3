//! Creditcoin L1 Outbox event listener (confluence §7.3 A3 / §6.8).
//!
//! Polls `eth_getLogs` for candidate `MessagePublished` events, authenticates each emitter against
//! Discovery at its finalized source block, and emits an [`IndexedMessage`] carrying the
//! `messageId` the attestor will sign directly (asc-contracts #54 — see `write_ability::hash`).
//! Every registered Outbox is covered by the same durable block cursor, independent of defaults.
//! Historical `eth_call` support is required; unavailable history stops progress rather than
//! dropping messages or trusting the permissionless factory. Authority is evaluated at block end,
//! matching Discovery's source-block removal boundaries (the effective block is excluded).
//!
//! Finality: the Outbox lives on Creditcoin L1, which has deterministic GRANDPA finality, so events
//! are surfaced up to the **finalized head** ([`FinalityPolicy::Finalized`]) — a finalized block
//! cannot be reorged out from under a signed vote (§6.8). A stalled or unavailable finalized head
//! pauses signing; an RPC failure must never weaken the finality requirement.
//! Polling (rather than `eth_subscribe`) avoids the silent-stream-stall failure mode, matching the
//! relayer.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::rpc::types::{Filter, Log};
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use write_ability::abi::IOutbox;

use super::cursor::CursorStore;
use super::resolver::{self, ResolvedRoute};

/// Poll cadence for `eth_getLogs`.
pub const DEFAULT_POLL_INTERVAL_SECS: u64 = 6;

/// Per-RPC deadline. This deliberately wraps only network calls, never delivery into the bounded
/// signing channel. A dense but valid log range may take longer than this to drain under
/// backpressure; cancelling mid-drain would leave the block cursor before the chunk and replay the
/// same prefix forever. Network calls still need a deadline so a black-holed provider cannot wedge
/// the listener.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Consecutive *stalled* polls before the listener declares its provider dead and rebuilds it in
/// place via the `reconnect` hook handed to [`watch`]. Only a poll that errored **and** made no
/// forward progress counts (see [`next_failure_count`]); a slow backfill that keeps advancing the
/// scan resets the budget, so catching up across many polls never trips this.
///
/// This is the reconnect story for the write-ability EVM provider: unlike the block-attestation
/// path (wrapped in the reconnecting `eth::Client`), this provider is a bare alloy connection whose
/// pubsub service exits permanently after a single failed reconnect (`backend connection task has
/// stopped`), so a routine RPC blip would otherwise silently kill message voting for the process
/// lifetime (C1). The listener used to return `Err` here and let the supervisor restart the whole
/// attestor — which took block attestation down with it and, because every attestor in a cluster
/// shares one RPC endpoint, killed whole clusters within seconds of each other (31 Aug 2026: all
/// six kc/we attestors, two deliveries that day at exactly quorum). Rebuilding the connection in
/// place is the same fix the Outbox resolver got in #1304. At the 6s cadence this rides out ~1
/// minute of fast errors, or (with `RPC_TIMEOUT`) a few minutes of a black-holed endpoint, before
/// the first rebuild attempt; a failed rebuild keeps the old handle and tries again after another
/// full budget.
const MAX_CONSECUTIVE_POLL_FAILURES: u32 = 10;

/// Blocks to rewind a *persisted* cursor by on resume. The cursor is saved once a poll's range has
/// been handed to the signing pipeline, not once each vote is durably gossiped (see [`super::cursor`]),
/// so a crash between the save and the gossip would otherwise leave the resumed cursor *past* votes
/// that never left the process. Rewinding by this margin re-scans that window on boot; re-signing is
/// harmless (the aggregator dedups by signer, the relayer dedups votes). Sized to comfortably cover
/// one poll's in-flight range plus finality lag, while keeping the restart re-scan cheap.
const CURSOR_RESUME_LOOKBACK_BLOCKS: u64 = 256;

/// Max block span per `eth_getLogs` request. The scan window can grow large — e.g. a long
/// Outbox-resolve wait leaves a wide gap between `last_seen` and the finalized tip on first poll —
/// and a single `eth_getLogs` over an unbounded span exceeds most RPC providers' range limits,
/// failing every poll until the failure budget restarts the task (which re-seeds from a fresh head
/// and permanently skips the unscanned span). Chunk the scan into ranges of this size instead; 2000
/// blocks is comfortably within the common provider caps (Alchemy/Infra allow ≥2k per query).
const MAX_LOG_BLOCK_RANGE: u64 = 2000;

/// How long the finalized head may stay frozen before we log a stalled-finality warning.
/// This is only an operational alert: elapsed time never authorizes unfinalized messages.
const FINALITY_STALL_TIMEOUT: Duration = Duration::from_secs(600);

/// The finality policy for the Outbox source chain (Creditcoin L1).
#[derive(Clone, Copy, Debug)]
pub enum FinalityPolicy {
    /// Sign up to the chain's GRANDPA-**finalized** head (exact, reorg-proof) — the production
    /// policy for Creditcoin. Pauses if finality is unavailable or stalled.
    Finalized,
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
    stalled: bool,
}

impl FinalityTracker {
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            last_finalized: None,
            last_advance: now,
            stalled: false,
        }
    }
}

/// Decide the highest block to sign up to. Pure so the finality policy is unit-testable without an
/// RPC. Elapsed time and the tip never advance the finalized-policy boundary. A missing or older
/// response preserves the last known finalized height, including across provider reconnections.
fn pick_to_block(
    finalized: Option<u64>,
    tip: u64,
    policy: &FinalityPolicy,
    tracker: &mut FinalityTracker,
    now: Instant,
) -> u64 {
    match *policy {
        FinalityPolicy::Depth(depth) => {
            tracker.stalled = false;
            tip.saturating_sub(depth)
        }
        FinalityPolicy::Finalized => {
            if let Some(f) = finalized {
                if tracker.last_finalized.is_none_or(|prev| f > prev) {
                    tracker.last_finalized = Some(f);
                    tracker.last_advance = now;
                }
            }
            tracker.stalled = now.duration_since(tracker.last_advance) >= FINALITY_STALL_TIMEOUT;
            tracker.last_finalized.unwrap_or(0)
        }
    }
}

/// A finalized `MessagePublished` the attestor should vote on.
#[derive(Clone, Debug)]
pub struct IndexedMessage {
    /// Outbox `messageId` — also the digest the attestor signs directly (asc-contracts #54:
    /// `Inbox.validateVotes` takes `messageId` itself, which is already cryptographically bound
    /// to the emitter/outbox/sequence/payload/sourceChainId by `OutboxTypes.computeMessageId` at
    /// emission time, so nothing needs re-deriving here).
    pub message_id: B256,
    /// Actual Outbox that emitted the message, authenticated at its finalized source block.
    pub outbox: Address,
    /// Signing domain captured when this message was indexed. A governance key change must not
    /// permit a buffered message from the previous route to be signed under the new configuration.
    pub destination_chain_key: B256,
}

/// Next value of the consecutive-poll-failure budget after one poll.
///
/// A poll that either completed (`poll_ok`) or advanced the scan (`made_progress`) resets the budget
/// to zero; only a fully *stalled* poll — errored **and** zero forward progress — increments it.
/// This is what stops a slow-but-progressing backfill (which may encounter a later RPC failure after
/// advancing earlier chunks) from tripping [`MAX_CONSECUTIVE_POLL_FAILURES`] and rebuilding a
/// provider that is in fact healthy. Pure so the rule is unit-testable without an RPC or timers.
fn next_failure_count(prev: u32, poll_ok: bool, made_progress: bool) -> u32 {
    if poll_ok || made_progress {
        0
    } else {
        prev.saturating_add(1)
    }
}

/// Watch every historically authorized Outbox for the route until `token` fires. Sends each finalized message on `tx`.
///
/// `cursor` persists the scan position (`last_seen`) across restarts: on boot the persisted value
/// is preferred over `start_block`/head so a restart resumes exactly where it left off, and after
/// every poll that advances the scan the new position is saved.
///
/// `shared_provider` is the write-ability task's one Creditcoin L1 connection, shared with the
/// Outbox rotation monitor and the reobservation worker. alloy provider clones share a single
/// pubsub backend, so when that backend dies every clone dies with it. The listener is the one
/// loop that counts stalls, so it is the one that heals: after [`MAX_CONSECUTIVE_POLL_FAILURES`]
/// stalled polls it calls `reconnect`, swaps the fresh connection into its own hot path, and
/// publishes it through the channel so the siblings pick it up on their next use — nobody leaves
/// their loop (the scan cursor, finality tracker and cancellation wiring all survive). `reconnect`
/// must produce the same provider type; in production that is `connect_l1_provider` bound to the
/// configured RPC URL.
#[allow(clippy::too_many_arguments)]
pub async fn watch<P, R, Fut>(
    shared_provider: watch::Sender<P>,
    reconnect: R,
    resolved: ResolvedRoute,
    _block_confirmation_depth: u64,
    start_block: Option<u64>,
    cursor: CursorStore,
    tx: mpsc::Sender<IndexedMessage>,
    token: CancellationToken,
) -> Result<()>
where
    P: Provider + Clone,
    R: Fn() -> Fut,
    Fut: Future<Output = Result<P>>,
{
    tracing::warn!(
        chain_key = resolved.chain_key,
        "Outbox authorization requires archive state and historical eth_call for the entire \
         scan and recovery range, including the chain-info precompile and Discovery registry; \
         unavailable history pauses the affected range. Before migrating a legacy cursor, \
         configure start_block or expect a replay from genesis"
    );
    // Local handle for the hot path; the channel is only touched on a rebuild. Cloning an alloy
    // provider is an `Arc` bump. Start from whatever is current, which after a rotation re-spawn
    // may already be a rebuilt connection rather than the boot-time one.
    let mut provider = shared_provider.borrow().clone();
    // Cursor precedence: a persisted position wins over `start_block`/head so a restart resumes
    // rather than skipping down-time messages (default config) or replaying the whole history from
    // `start_block`. To force a different start position, remove the cursor file.
    let mut last_seen = if let Some(persisted) = cursor.load() {
        // Read the live head to sanity-clamp the persisted position. A cursor *ahead* of the current
        // head (chain rolled back / resynced to a shorter chain, or a file copied from another node)
        // would otherwise leave `from_block = last_seen + 1 > tip` and silently scan nothing forever
        // (bugbot: stale cursor stalls scanning). On a transient head-read failure, fall back to the
        // persisted value unclamped rather than fail boot.
        let head = match tokio::time::timeout(RPC_TIMEOUT, provider.get_block_number()).await {
            Ok(Ok(head)) => head,
            Ok(Err(err)) => {
                tracing::warn!(%err, "could not clamp persisted Outbox cursor to live head");
                persisted
            }
            Err(_) => {
                tracing::warn!("timed out clamping persisted Outbox cursor to live head");
                persisted
            }
        };
        let clamped = persisted.min(head);
        // Rewind by a bounded lookback so a crash between enqueuing a range's votes and gossiping them
        // re-scans that window (at-least-once; downstream dedups). See CURSOR_RESUME_LOOKBACK_BLOCKS.
        let resume = clamped
            .saturating_sub(CURSOR_RESUME_LOOKBACK_BLOCKS)
            .max(cursor.scan_floor());
        tracing::info!(
            persisted,
            head,
            resume_from = resume + 1,
            "⏮️ resuming message-attestation scan from persisted Outbox cursor (clamped to tip, rewound by lookback)"
        );
        resume
    } else if let Some(start) = start_block {
        tracing::info!(
            start_block = start,
            "⏮️ no persisted Outbox cursor; starting message-attestation scan from configured block"
        );
        start.saturating_sub(1)
    } else {
        let head = tokio::time::timeout(RPC_TIMEOUT, provider.get_block_number())
            .await
            .context("timed out reading Creditcoin L1 chain head")?
            .context("failed to read Creditcoin L1 chain head")?;
        tracing::info!(
            head,
            "⏮️ no persisted Outbox cursor or start_block; scanning from current head (future messages only)"
        );
        head
    };

    let mut tick = tokio::time::interval(Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Creditcoin L1 has deterministic GRANDPA finality, so the normal signing boundary is the
    // finalized head (reorg-proof). The legacy confirmation-depth argument is retained for callers,
    // but never substitutes a probabilistic boundary when deterministic finality is unavailable.
    let policy = FinalityPolicy::Finalized;
    let mut finality = FinalityTracker::new(Instant::now());

    tracing::info!(
        chain_key = resolved.chain_key,
        ?resolved.destination_chain_key,
        creditcoin_chain_id = resolved.creditcoin_chain_id,
        "📡 message-attestation Outbox listener online (signing finalized head only)"
    );

    let mut consecutive_failures: u32 = 0;
    loop {
        tokio::select! {
            () = token.cancelled() => {
                tracing::info!("🛑 Outbox listener exiting on cancel");
                return Ok(());
            }
            _ = tick.tick() => {
                let prev_seen = last_seen;
                // Individual network calls inside `poll_once` are deadline-bounded. Do not wrap the
                // whole poll: delivery to `tx` is intentionally allowed to wait for downstream
                // capacity so a dense chunk is delivered exactly once before its cursor advances.
                //
                // That wait is unbounded, though, and `poll_once` is awaited inside this arm's body
                // rather than as a `select!` branch — so without racing the cancel token here, a
                // consumer that is wedged but still holding the receiver would make the listener
                // ignore shutdown entirely (the removed whole-poll timeout used to cap that at 30s).
                // Abandoning a poll mid-drain is safe: the cursor only advances after a full chunk,
                // and re-scanning a range on the next boot is the documented at-least-once contract.
                let outcome = tokio::select! {
                    () = token.cancelled() => {
                        tracing::info!("🛑 Outbox listener exiting on cancel (mid-poll)");
                        return Ok(());
                    }
                    outcome = poll_once(
                        &provider, &resolved, &policy, &mut finality, &mut last_seen, &tx,
                    ) => outcome,
                };

                // Did the scan advance this poll? `poll_once` bumps `last_seen` after each
                // successful chunk, so this is true even when the poll ultimately errored/timed out
                // part-way through a wide backfill. Drives both cursor persistence and — crucially —
                // the failure budget below.
                let made_progress = last_seen != prev_seen;

                // Persist any forward progress this poll made — including a *partial* advance before
                // an error mid-way through a chunked backfill. At-least-once by design: re-scanning
                // the in-flight range after a crash is harmless (downstream dedups), so we save
                // whenever `last_seen` moved rather than gating on `Ok`. A persistence failure is
                // logged but never fails the poll — losing the volume shouldn't take signing down;
                // the next advance retries the write.
                if made_progress {
                    if let Err(err) = cursor.save(last_seen) {
                        tracing::warn!(
                            %err, last_seen,
                            "failed to persist Outbox cursor; will retry on next advance"
                        );
                    }
                }

                // Progress-aware failure budget: a poll that either completed or *advanced the scan*
                // resets the budget; only a fully stalled poll (errored AND zero forward progress)
                // counts toward the rebuild threshold. A wide backfill (far-behind start_block, or a
                // long Outbox-resolve wait) can legitimately advance several chunks before a later
                // RPC fails — counting that progressing-but-unfinished poll as a failure would
                // trip `MAX_CONSECUTIVE_POLL_FAILURES` and tear down a healthy connection even
                // while it was catching up the entire time.
                consecutive_failures =
                    next_failure_count(consecutive_failures, outcome.is_ok(), made_progress);
                match outcome {
                    Ok(()) => {}
                    // Errored but the scan advanced — a slow-but-progressing backfill, not a fault.
                    // Budget already reset above; keep going.
                    Err(err) if made_progress => {
                        tracing::info!(
                            last_seen, %err,
                            "🐢 outbox backfill advanced but hit an RPC failure mid-scan; \
                             continuing (not counted as a failure)"
                        );
                    }
                    // Stalled poll (no progress): the budget was incremented above.
                    Err(err) => {
                        if consecutive_failures >= MAX_CONSECUTIVE_POLL_FAILURES {
                            // The bare alloy connection does not come back on its own once its
                            // pubsub task has exited (C1), so swap it for a fresh one here rather
                            // than exiting and taking block attestation down with us. Reset the
                            // budget either way: a failed rebuild keeps the old handle and earns
                            // another full window before the next attempt, so a long RPC outage
                            // produces a warning every ~minute instead of a crash loop.
                            consecutive_failures = 0;
                            tracing::warn!(
                                error = %format!("{err:#}"),
                                stalled_polls = MAX_CONSECUTIVE_POLL_FAILURES,
                                last_seen,
                                "🔌 outbox poll stalled for a full window — rebuilding the Creditcoin L1 EVM provider in place"
                            );
                            match reconnect().await {
                                Ok(fresh) => {
                                    // Publish first so the rotation monitor and reobservation
                                    // worker stop using the dead backend on their next tick, then
                                    // take it for our own hot path.
                                    shared_provider.send_replace(fresh.clone());
                                    provider = fresh;
                                    tracing::info!(
                                        last_seen,
                                        "🔌 Creditcoin L1 EVM provider rebuilt and shared with the rotation monitor and reobservation worker; resuming Outbox scan from cursor"
                                    );
                                }
                                Err(rebuild_err) => {
                                    tracing::warn!(
                                        error = %format!("{rebuild_err:#}"),
                                        "🔌 could not rebuild the Creditcoin L1 EVM provider; keeping the current handle and retrying"
                                    );
                                }
                            }
                        } else {
                            tracing::warn!(
                                %err,
                                consecutive_failures,
                                max = MAX_CONSECUTIVE_POLL_FAILURES,
                                "outbox poll stalled with no progress; will retry"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Run a single poll iteration, signing up to the boundary chosen by `policy` (the finalized head,
/// or an explicitly selected test-chain depth policy — see [`pick_to_block`]). Exposed (beyond the internal
/// [`watch`] loop) so the anvil e2e test can drive polling deterministically.
pub async fn poll_once<P: Provider>(
    provider: &P,
    resolved: &ResolvedRoute,
    policy: &FinalityPolicy,
    finality: &mut FinalityTracker,
    last_seen: &mut u64,
    tx: &mpsc::Sender<IndexedMessage>,
) -> Result<()> {
    // A failed/null finalized read is a failed poll, so the existing reconnect budget can heal a
    // broken provider. Neither the cursor nor the remembered finality boundary advances on failure.
    // The finalized policy does not even read the tip: it is not evidence of deterministic finality.
    let (finalized, tip) = match policy {
        FinalityPolicy::Finalized => {
            let block = tokio::time::timeout(
                RPC_TIMEOUT,
                provider.get_block_by_number(BlockNumberOrTag::Finalized),
            )
            .await
            .context("finalized-head read timed out; signing paused")?
            .context("finalized-head read failed; signing paused")?
            .context("finalized head unavailable; signing paused")?;
            if finality
                .last_finalized
                .is_some_and(|previous| block.header.number < previous)
            {
                anyhow::bail!("finalized head regressed; signing paused");
            }
            (Some(block.header.number), 0)
        }
        FinalityPolicy::Depth(_) => (
            None,
            tokio::time::timeout(RPC_TIMEOUT, provider.get_block_number())
                .await
                .context("eth_blockNumber timed out")??,
        ),
    };

    let was_stalled = finality.stalled;
    let to_block = pick_to_block(finalized, tip, policy, finality, Instant::now());
    if finality.stalled && !was_stalled {
        tracing::warn!(
            finalized = to_block,
            "⚠️ source finality stalled — waiting for finalized head to advance"
        );
    } else if !finality.stalled && was_stalled {
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
    let mut span = MAX_LOG_BLOCK_RANGE;
    while from_block <= to_block {
        let chunk_to = to_block.min(from_block.saturating_add(span - 1));
        match scan_range(provider, resolved, from_block, chunk_to, tx).await {
            Ok(()) => {
                *last_seen = chunk_to;
                from_block = chunk_to + 1;
                // Grow back after a successful chunk so one dense prefix does not turn the rest
                // of a wide backfill into a crawl of tiny queries (bugbot). Doubling rather than
                // snapping to the maximum keeps a still-dense region from paying a failed
                // oversized query on every step.
                span = span.saturating_mul(2).min(MAX_LOG_BLOCK_RANGE);
            }
            Err(err) if err.is::<LogRangeTooLarge>() => {
                // Process and persist each smaller prefix before fetching the next. Collecting
                // every split result into a single original-range Vec would let spam bypass the
                // provider's cap into unbounded memory growth across thousands of source blocks.
                span = (chunk_to - from_block).div_ceil(2);
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

#[derive(Debug)]
struct LogRangeTooLarge;
impl std::fmt::Display for LogRangeTooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("source log range exceeds the provider result limit")
    }
}
impl std::error::Error for LogRangeTooLarge {}

/// Fetch one bounded candidate range. The caller shrinks a capped multi-block range and drains
/// each successful prefix before continuing. A single block uses receipts, avoiding a permanent
/// stall on unauthorized spam while keeping the fallback bounded to one source block.
pub(super) async fn fetch_message_logs<P: Provider>(
    provider: &P,
    from_block: u64,
    to_block: u64,
    message_id: Option<B256>,
) -> Result<Vec<Log>> {
    let mut filter = Filter::new()
        .event_signature(IOutbox::MessagePublished::SIGNATURE_HASH)
        .from_block(from_block)
        .to_block(to_block);
    if let Some(id) = message_id {
        filter = filter.topic1(id);
    }
    match tokio::time::timeout(RPC_TIMEOUT, provider.get_logs(&filter))
        .await
        .context("eth_getLogs timed out")?
    {
        Ok(found) => Ok(found),
        Err(err) if log_limit_error(&err.to_string()) && from_block < to_block => {
            Err(LogRangeTooLarge.into())
        }
        Err(err) if log_limit_error(&err.to_string()) => {
            message_logs_from_receipts(provider, from_block, message_id).await
        }
        Err(err) => Err(err).context("eth_getLogs failed"),
    }
}

fn log_limit_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    // Frontier's max_past_logs error, plus common hosted-provider equivalents.
    message.contains("query returned more than")
        || message.contains("too many results")
        || message.contains("too many logs")
        || message.contains("log response size exceeded")
}

async fn message_logs_from_receipts<P: Provider>(
    provider: &P,
    number: u64,
    message_id: Option<B256>,
) -> Result<Vec<Log>> {
    let block = tokio::time::timeout(
        RPC_TIMEOUT,
        provider.get_block_by_number(BlockNumberOrTag::Number(number)),
    )
    .await
    .context("receipt fallback block lookup timed out")??
    .context("receipt fallback block missing")?;
    anyhow::ensure!(
        block.header.number == number,
        "receipt fallback returned the wrong block"
    );
    let mut logs = Vec::new();
    for hash in block.transactions.hashes() {
        let receipt = tokio::time::timeout(RPC_TIMEOUT, provider.get_transaction_receipt(hash))
            .await
            .context("receipt fallback lookup timed out")??
            .context("receipt fallback transaction receipt missing")?;
        anyhow::ensure!(
            receipt.transaction_hash == hash
                && receipt.block_number == Some(number)
                && receipt.block_hash == Some(block.header.hash),
            "receipt fallback returned inconsistent provenance"
        );
        for log in receipt.inner.logs() {
            anyhow::ensure!(
                log.block_number == Some(number)
                    && log.block_hash == receipt.block_hash
                    && log.transaction_hash == Some(hash)
                    && !log.removed,
                "receipt fallback log has inconsistent provenance"
            );
            if log.topic0() == Some(&IOutbox::MessagePublished::SIGNATURE_HASH)
                && message_id.is_none_or(|id| log.topics().get(1) == Some(&id))
            {
                logs.push(log.clone());
            }
        }
    }
    Ok(logs)
}

/// Fetch + index `MessagePublished` logs in the inclusive block range `[from_block, to_block]`.
/// Returns `Err` (without the caller advancing `last_seen`) on an RPC failure or an ABI-mismatch
/// decode error, so the exact range is retried rather than stepped over.
async fn scan_range<P: Provider>(
    provider: &P,
    resolved: &ResolvedRoute,
    from_block: u64,
    to_block: u64,
    tx: &mpsc::Sender<IndexedMessage>,
) -> Result<()> {
    let logs = fetch_message_logs(provider, from_block, to_block, None).await?;

    // Cache only within this bounded chunk, and key every decision by its historical block.
    // Querying all candidate emitters avoids missing an Outbox's complete registration/removal
    // lifecycle between polls, or an old Discovery registry replaced during downtime.
    let mut discoveries = HashMap::new();
    let mut memberships = HashMap::new();
    for log in logs {
        let block = log
            .block_number
            .context("MessagePublished has no block number")?;
        anyhow::ensure!(
            !log.removed && (from_block..=to_block).contains(&block),
            "MessagePublished is removed or outside the requested finalized range"
        );
        let discovery = match discoveries.get(&block) {
            Some(discovery) => *discovery,
            None => {
                let discovery = tokio::time::timeout(
                    RPC_TIMEOUT,
                    resolver::discovery_at(provider, resolved, block),
                )
                .await
                .context("historical Discovery lookup timed out")??;
                discoveries.insert(block, discovery);
                discovery
            }
        };
        let Some(discovery) = discovery else { continue };
        let outbox = log.address();
        let authorized = match memberships.get(&(block, outbox)) {
            Some(authorized) => *authorized,
            None => {
                let authorized = tokio::time::timeout(
                    RPC_TIMEOUT,
                    resolver::authorized_at(provider, resolved, discovery, outbox, block),
                )
                .await
                .context("historical Outbox membership lookup timed out")??;
                memberships.insert((block, outbox), authorized);
                authorized
            }
        };
        if !authorized {
            continue;
        }
        match IOutbox::MessagePublished::decode_log_validate(&log.inner) {
            Ok(decoded) => {
                // `sequence`/`emitterAddress`/`payload` are already cryptographically bound into
                // `messageId` by `OutboxTypes.computeMessageId` at emission time (asc-contracts
                // #54); we only need `messageId` itself to sign and `outbox` for the trust
                // boundary already enforced above. `sequence` is decoded (it shifted the ABI
                // layout of `canAck`/`payload`) but otherwise unused here.
                let indexed = IndexedMessage {
                    message_id: decoded.data.messageId,
                    outbox,
                    destination_chain_key: resolved.destination_chain_key,
                };
                tracing::debug!(
                    message_id = %indexed.message_id,
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
pub(super) mod test_rpc {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use parking_lot::Mutex;
    use serde_json::{json, Value};
    use std::sync::Arc;

    #[derive(Default)]
    pub struct RpcState {
        pub finalized_response: Value,
        pub requests: Vec<Value>,
    }

    pub struct RpcMock {
        pub url: url::Url,
        pub state: Arc<Mutex<RpcState>>,
        server: tokio::task::JoinHandle<()>,
    }

    impl RpcMock {
        pub async fn start() -> Self {
            async fn handle(
                State(state): State<Arc<Mutex<RpcState>>>,
                Json(request): Json<Value>,
            ) -> Json<Value> {
                let mut state = state.lock();
                state.requests.push(request.clone());
                let mut response = match request["method"].as_str().unwrap() {
                    "eth_getBlockByNumber" => {
                        assert_eq!(request["params"][0], "finalized");
                        state.finalized_response.clone()
                    }
                    // The old fallback would fetch this high tip and then scan unfinalized logs.
                    "eth_blockNumber" => json!({"result": "0x2710"}),
                    "eth_getLogs" => json!({"result": []}),
                    method => panic!("unexpected method {method}"),
                };
                response["jsonrpc"] = json!("2.0");
                response["id"] = request["id"].clone();
                Json(response)
            }

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap())
                .parse()
                .unwrap();
            let state = Arc::new(Mutex::new(RpcState {
                finalized_response: json!({"result": null}),
                requests: Vec::new(),
            }));
            let router = Router::new()
                .route("/", post(handle))
                .with_state(state.clone());
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            Self { url, state, server }
        }

        pub fn set_finalized(&self, height: u64) {
            let mut block: alloy::rpc::types::eth::Block = Default::default();
            block.header.inner.number = height;
            self.state.lock().finalized_response = json!({"result": block});
        }

        pub fn resolved() -> ResolvedRoute {
            ResolvedRoute {
                chain_key: 1,
                destination_chain_key: B256::repeat_byte(2),
                creditcoin_chain_id: 102030,
            }
        }
    }

    impl Drop for RpcMock {
        fn drop(&mut self) {
            self.server.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(t0: Instant) -> FinalityTracker {
        FinalityTracker::new(t0)
    }

    #[test]
    fn ok_poll_resets_failure_budget() {
        // A completed poll clears any accumulated stalls, with or without new blocks.
        assert_eq!(next_failure_count(5, true, false), 0);
        assert_eq!(next_failure_count(5, true, true), 0);
    }

    #[test]
    fn progressing_backfill_never_counts_as_failure() {
        // The core fix: an errored/timed-out poll that still advanced the scan resets the budget,
        // so a slow backfill spanning many polls can't trip MAX_CONSECUTIVE_POLL_FAILURES.
        let mut failures = 0;
        for _ in 0..(MAX_CONSECUTIVE_POLL_FAILURES + 5) {
            failures = next_failure_count(failures, false, true);
            assert_eq!(failures, 0);
        }
    }

    #[test]
    fn stalled_polls_accumulate_to_the_limit() {
        // Errored AND no progress is a real stall: it climbs until the restart threshold.
        let mut failures = 0;
        for expected in 1..=MAX_CONSECUTIVE_POLL_FAILURES {
            failures = next_failure_count(failures, false, false);
            assert_eq!(failures, expected);
        }
        assert!(failures >= MAX_CONSECUTIVE_POLL_FAILURES);
    }

    #[test]
    fn one_progressing_poll_clears_a_stall_run() {
        // A stall streak that later makes progress is forgiven — the budget resets rather than
        // carrying prior stalls forward into an unrelated slow-but-advancing stretch.
        let failures = next_failure_count(MAX_CONSECUTIVE_POLL_FAILURES - 1, false, true);
        assert_eq!(failures, 0);
    }

    #[test]
    fn depth_policy_uses_tip_minus_depth() {
        let t0 = Instant::now();
        let mut tr = tracker(t0);
        assert_eq!(
            pick_to_block(None, 110, &FinalityPolicy::Depth(3), &mut tr, t0),
            107
        );
        assert!(!tr.stalled);
        // depth 0 = index up to tip (the anvil e2e case).
        assert_eq!(
            pick_to_block(None, 110, &FinalityPolicy::Depth(0), &mut tr, t0),
            110
        );
    }

    #[test]
    fn finalized_primary_uses_finalized_head() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized;
        let mut tr = tracker(t0);
        // First observation + subsequent advance both sign the finalized head, not tip-depth.
        assert_eq!(pick_to_block(Some(100), 110, &pol, &mut tr, t0), 100);
        assert!(!tr.stalled);
        assert_eq!(
            pick_to_block(Some(105), 120, &pol, &mut tr, t0 + Duration::from_secs(6)),
            105
        );
        assert!(!tr.stalled);
    }

    #[test]
    fn lagging_but_not_stalled_stays_at_finalized() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized;
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(Some(100), 110, &pol, &mut tr, t0), 100);
        // Finalized frozen at 100 while tip climbs, but within the stall window → still 100.
        let within = t0 + FINALITY_STALL_TIMEOUT - Duration::from_secs(1);
        assert_eq!(pick_to_block(Some(100), 200, &pol, &mut tr, within), 100);
        assert!(!tr.stalled);
    }

    #[test]
    fn stalled_finality_never_advances_past_finalized_head() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized;
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(Some(100), 110, &pol, &mut tr, t0), 100);
        // Even hours of stalled finality and an advancing tip never authorize another block.
        let past = t0 + Duration::from_secs(24 * 60 * 60);
        assert_eq!(pick_to_block(Some(100), 200, &pol, &mut tr, past), 100);
        assert!(tr.stalled);
        // Then finality recovers (advances) → back to signing the finalized head.
        assert_eq!(
            pick_to_block(Some(210), 220, &pol, &mut tr, past + Duration::from_secs(6)),
            210
        );
        assert!(!tr.stalled);
    }

    #[test]
    fn no_finalized_head_does_not_authorize_any_new_block() {
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized;
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(None, 100, &pol, &mut tr, t0), 0);
        assert!(!tr.stalled);
    }

    #[test]
    fn missing_or_regressed_finalized_head_preserves_the_known_boundary() {
        // A prior good read established finalized head 100. Neither a missing head nor a stale
        // response is evidence authorizing signatures beyond it.
        let t0 = Instant::now();
        let pol = FinalityPolicy::Finalized;
        let mut tr = tracker(t0);
        assert_eq!(pick_to_block(Some(100), 100, &pol, &mut tr, t0), 100);
        // A missing tag keeps the known boundary, regardless of how far the tip advances.
        assert_eq!(pick_to_block(None, 101, &pol, &mut tr, t0), 100);
        assert_eq!(pick_to_block(None, 10_000, &pol, &mut tr, t0), 100);
        assert_eq!(pick_to_block(Some(99), 10_000, &pol, &mut tr, t0), 100);
    }

    #[tokio::test]
    async fn failed_finalized_reads_never_scan_and_recovery_resumes_from_cursor() {
        use alloy::providers::ProviderBuilder;
        use serde_json::json;
        let rpc = test_rpc::RpcMock::start().await;
        let provider = ProviderBuilder::new().connect_http(rpc.url.clone());
        let resolved = test_rpc::RpcMock::resolved();
        let mut finality = FinalityTracker::new(Instant::now());
        let mut last_seen = 90;
        let (tx, mut rx) = mpsc::channel(8);
        for failure in [
            json!({"result": null}),
            json!({"error": {"code": -32000, "message": "finality unavailable"}}),
        ] {
            rpc.state.lock().finalized_response = failure;
            assert!(poll_once(
                &provider,
                &resolved,
                &FinalityPolicy::Finalized,
                &mut finality,
                &mut last_seen,
                &tx
            )
            .await
            .is_err());
            assert_eq!(last_seen, 90);
            assert!(rx.try_recv().is_err());
        }
        assert!(rpc
            .state
            .lock()
            .requests
            .iter()
            .all(|r| r["method"] == "eth_getBlockByNumber"));
        rpc.set_finalized(100);
        poll_once(
            &provider,
            &resolved,
            &FinalityPolicy::Finalized,
            &mut finality,
            &mut last_seen,
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(last_seen, 100);
        let scan = rpc.state.lock().requests.last().unwrap().clone();
        assert_eq!(scan["method"], "eth_getLogs");
        assert_eq!(scan["params"][0]["fromBlock"], "0x5b");
        assert_eq!(scan["params"][0]["toBlock"], "0x64");

        // A later stale provider response cannot authorize scanning against that provider.
        rpc.set_finalized(99);
        assert!(poll_once(
            &provider,
            &resolved,
            &FinalityPolicy::Finalized,
            &mut finality,
            &mut last_seen,
            &tx
        )
        .await
        .is_err());
        assert_eq!(last_seen, 100);
        assert_eq!(finality.last_finalized, Some(100));
    }
}
