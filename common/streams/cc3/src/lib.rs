//! CC3 finalized-block stream with gap-resistant emission.
//!
//! `subxt::OnlineClient::blocks().subscribe_finalized()` is the source. It can drop blocks for
//! two reasons we have to handle here:
//!
//!   - **Reconnect gap.** WS dies → we open a new subscription → the new subscription starts
//!     at whatever block is "head" *now*, missing everything between `latest` and `head`.
//!   - **In-stream gap.** Even on a live subscription, substrate sometimes yields block `N`
//!     and then jumps to `N+2` without emitting `N+1` (typically when the node fell briefly
//!     behind and skipped re-broadcasting an intermediate block).
//!
//! Both cases are unified by walking back from the received block via `parent_hash` until we
//! land at `latest + 1`, accumulating events in `backfill`, then draining oldest-first. The
//! `cc3.reconnect()` path is only invoked when subxt errors — gap handling is the same
//! mechanism in both cases.
//!
//! `cc3.reconnect()` carries its own shared backoff (`bcf6b8de`), so this file doesn't
//! re-implement retry timing. Re-subscribe attempts are *unbounded* — RPC outages can be long
//! (hours), and crashing the stream would lose downstream attestation work that could otherwise
//! resume cleanly. The sleep below each failed attempt is the cancellation point: shutdown
//! drops the stream future, which drops the loop.

mod error;
pub use error::Error;

/// Cap individual re-subscribe backoff at 30s. Doubles on each failure starting at 500ms.
const RESUBSCRIBE_BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(30);
const RESUBSCRIBE_BACKOFF_START: std::time::Duration = std::time::Duration::from_millis(500);

/// Minimum interval between consecutive parent-block fetches during backfill. Set to cap the
/// backfill load on the cc3 RPC — a long gap (post-outage recovery) otherwise issues sequential
/// requests as fast as the RPC can answer, which on a recovering node is enough to push it back
/// over. 100ms = ~10 fetches/sec is the default; tune via [`ConfigBuilder::with_backfill_min_interval`].
const DEFAULT_BACKFILL_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Hard cap on the parent-walk backfill gap. The walk buffers every `(height, events)` in the
/// `backfill` Vec before draining, so a very large — but still *servable* — gap (a long outage
/// recovering on an archive / large-pruning node, where the state is not pruned so
/// [`is_permanent_backfill_error`] never trips) would materialize the whole gap at once and can
/// OOM the pod mid-recovery. When the gap exceeds this, the stream ends instead: the consumer
/// treats that as fatal and restarts, re-seeding from the current head with no backfill
/// (accepting the skip). That is consistency-equivalent to the pruned-state re-seed the stream
/// already relies on — just controlled, rather than an OOM kill. ~10k blocks comfortably covers a
/// realistic reconnect/outage while keeping the buffer bounded.
const MAX_BACKFILL_BLOCKS: u64 = 10_000;

/// Default progress deadline: how long the finalized subscription may stay silent before the
/// stream asks the node point-to-point where its finalized head is. Creditcoin finalizes a
/// block every 5–15 s depending on the network, so 90 s is several missed blocks — long enough
/// that a slow-but-live chain never trips it, short enough that a dead subscription is
/// replaced well inside any downstream freshness window. Override with
/// [`ConfigBuilder::with_progress_timeout`].
const DEFAULT_PROGRESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Live progress of a [`StreamCC3`], for health reporting. Shared with the consumer through
/// [`ConfigBuilder::with_progress`]; all fields are plain atomics.
#[derive(Debug, Default)]
pub struct Progress {
    height: std::sync::atomic::AtomicU64,
    advanced_at_unix_ms: std::sync::atomic::AtomicU64,
    silent_recoveries: std::sync::atomic::AtomicU64,
}

impl Progress {
    /// Record that `height` has been processed. Driven by the stream; public so consumers
    /// and tests can seed it.
    pub fn note(&self, height: u64) {
        use std::sync::atomic::Ordering;
        self.height.store(height, Ordering::Release);
        self.advanced_at_unix_ms
            .store(now_unix_ms(), Ordering::Release);
    }

    fn note_silent_recovery(&self) {
        self.silent_recoveries
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// Highest finalized height the stream has yielded, if any.
    #[must_use]
    pub fn height(&self) -> Option<u64> {
        use std::sync::atomic::Ordering;
        (self.advanced_at_unix_ms.load(Ordering::Acquire) != 0)
            .then(|| self.height.load(Ordering::Acquire))
    }

    /// Wall-clock unix millis when [`Self::height`] last advanced, if ever.
    #[must_use]
    pub fn advanced_at_unix_ms(&self) -> Option<u64> {
        let at = self
            .advanced_at_unix_ms
            .load(std::sync::atomic::Ordering::Acquire);
        (at != 0).then_some(at)
    }

    /// Seconds since the stream last yielded a finalized block, if it ever has.
    #[must_use]
    pub fn age_seconds(&self) -> Option<u64> {
        self.advanced_at_unix_ms()
            .map(|at| now_unix_ms().saturating_sub(at) / 1000)
    }

    /// How many times the watchdog replaced a subscription that had gone silent while the node
    /// kept finalizing. A rising count on a healthy node points at the RPC endpoint (or the
    /// path to it), not at the chain.
    #[must_use]
    pub fn silent_recoveries(&self) -> u64 {
        self.silent_recoveries
            .load(std::sync::atomic::Ordering::Acquire)
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
        .max(1)
}

/// Whether a block/events fetch failed *permanently* for the block being asked about: the node
/// has pruned the state (or never had the block) and no amount of reconnecting brings it back —
/// only an archive node could answer. This happens when an outage outlasts the node's pruning
/// horizon (~256 blocks on a default non-archive node), leaving the gap walk asking for blocks
/// whose state is gone. Retrying such errors forever wedges the stream silently: every retry's
/// `reconnect()` succeeds against the healthy node, so even a reconnect-aware watchdog reads
/// "actively reconnecting" indefinitely. The stream ends instead; the consumer treats that as
/// fatal and restarts, which re-seeds from the current head with no backfill.
///
/// String-matched because subxt surfaces no structured code for pruned state. Deliberately
/// narrow — matching only substrate's pruned/missing-block messages — so a transient error can
/// never be misclassified as permanent (the failure mode of a broad match); an unlisted
/// permanent message just falls back to today's behavior (retry forever).
fn is_permanent_backfill_error(err: &subxt::Error) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("state already discarded")
        || text.contains("unknownblock")
        || text.contains("unknown block")
        || text.contains("header was not found")
        || text.contains("body was not found")
}

#[derive(Debug, builder::Builder)]
pub struct Config {
    /// Shared client handle. Held as `Arc` (not a value `Client`) so the stream's
    /// `reconnect()` calls swap the connection and bump the reconnect timestamp on the *same*
    /// instance every other task and `/health` observe — a value clone gets an independent
    /// `ArcSwap` + reconnect state, so its recoveries would be invisible to the rest of the node.
    cc3: std::sync::Arc<cc_client::Client>,
    chain_keys: Vec<attestor_primitives::ChainKey>,
    /// Minimum interval between consecutive parent-block fetches during backfill — applies only
    /// to gap recovery, not to the live `subscribe_finalized` flow.
    #[default(DEFAULT_BACKFILL_MIN_INTERVAL)]
    backfill_min_interval: std::time::Duration,
    /// Progress watchdog: with no finalized block for this long, the stream reads the node's
    /// finalized head point-to-point. Node ahead of us → the subscription is dead even though
    /// the socket is up: reconnect and re-subscribe (the parent walk fills the gap). Node not
    /// ahead → finality itself is stalled: keep waiting and log. Probe failure → reconnect.
    #[default(DEFAULT_PROGRESS_TIMEOUT)]
    progress_timeout: std::time::Duration,
    /// Optional shared progress record for health reporting.
    #[default(None)]
    progress: Option<std::sync::Arc<Progress>>,
    /// Height the consumer's state is already consistent up to (typically the finalized
    /// height its startup snapshot was read at). Blocks at or below it are not yielded; the
    /// blocks between it and the first subscribed block are backfilled through the parent
    /// walk, so nothing that landed between snapshot and subscription is skipped. `None`
    /// starts from the first subscribed block.
    #[default(None)]
    resume_from: Option<u64>,
}

pub struct StreamCC3 {
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = StreamEvents> + Send>>,
}

impl StreamCC3 {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        let chain_keys: std::sync::Arc<[attestor_primitives::ChainKey]> = config.chain_keys.into();
        let cc3 = config.cc3;
        let backfill_min_interval = config.backfill_min_interval;
        let progress_timeout = config.progress_timeout;
        let progress = config.progress;
        let resume_from = config.resume_from;

        // Initial subscription + first-block seed, under the same unbounded
        // reconnect-and-re-subscribe policy as the steady-state repair loop below. A
        // single-attempt initial subscribe made a boot-time WS blip construction-fatal — a
        // pod-restart recovery for exactly the outage the stream already knows how to ride out
        // in place (and a crash-loop under a persistently flappy RPC). Cancellation point is
        // the sleep: callers race construction against the root token / the bounded shutdown
        // drain, so an endless outage cannot pin shutdown.
        let (finalized, first) = {
            let mut backoff = RESUBSCRIBE_BACKOFF_START;
            loop {
                let attempt = async {
                    let mut finalized = cc3
                        .api()
                        .blocks()
                        .subscribe_finalized()
                        .await
                        .map_err(Error::Subxt)?;
                    // Seed from the first block. An immediately-ended stream or a failed
                    // events fetch is the same transport blip as a failed subscribe — retry
                    // it, don't die on it. A subscription that never delivers is one too: a
                    // node accepts the subscribe, then nothing (seen on overloaded RPCs); the
                    // deadline turns that into a reconnect rather than a hang.
                    let first = tokio::time::timeout(progress_timeout, finalized.try_next())
                        .await
                        .map_err(|_| Error::NoProgress(progress_timeout))?
                        .map_err(Error::Subxt)?
                        .ok_or(Error::EndOfStream)?;
                    Ok::<_, Error>((finalized, first))
                };
                match attempt.await {
                    Ok(seed) => break seed,
                    Err(err) => {
                        tracing::warn!(
                            ?err,
                            "🛜 initial cc3 finalized subscription failed — reconnecting + retrying"
                        );
                        let _ = cc3.reconnect().await;
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
                    }
                }
            }
        };

        // Everything at or below `latest` is already known to the consumer. With `resume_from`
        // that is the consumer's snapshot height and the gap up to the first subscribed block
        // is backfilled by the parent walk below like any reconnect gap; without it the first
        // subscribed block is the first thing yielded (it goes through the same head path,
        // so its events fetch gets the same retry policy as every later block).
        let first_height = first.number() as u64;
        let mut latest = resume_from.unwrap_or_else(|| first_height.saturating_sub(1));
        if let Some(from) = resume_from {
            tracing::info!(
                from,
                first = first_height,
                "🛟 cc3 stream resuming from consumer snapshot"
            );
        }

        let stream = async_stream::stream! {
            let mut finalized = finalized;
            let mut pending = Some(first);
            // Reusable scratch buffer for the parent-walk backfill. Capacity tuned for
            // typical disconnects of <16 blocks; grows if needed.
            let mut backfill: Vec<(u64, subxt::events::Events<subxt::SubstrateConfig>)> =
                Vec::with_capacity(16);

            loop {
                // Progress watchdog. `try_next` alone can pend forever on a socket that is
                // open but no longer delivering (subscription dropped server-side, a proxy
                // that stopped forwarding, a peer that answers pings and nothing else).
                let next = if let Some(first) = pending.take() {
                    Ok(Some(first))
                } else { match tokio::time::timeout(progress_timeout, finalized.try_next()).await {
                    Ok(next) => next,
                    Err(_elapsed) => match cc3.finalized_head_number().await {
                        Ok(head) if head > latest => {
                            tracing::warn!(
                                latest, head, timeout = ?progress_timeout,
                                "🛜 finalized subscription silent while the node kept finalizing — replacing it"
                            );
                            if let Some(p) = &progress {
                                p.note_silent_recovery();
                            }
                            Err(subxt::Error::Other(format!(
                                "no finalized block for {progress_timeout:?} while node is at {head} (last seen {latest})"
                            )))
                        }
                        Ok(head) => {
                            tracing::warn!(
                                latest, head, timeout = ?progress_timeout,
                                "⏸️ no finalized block from the node either — finality stalled upstream, waiting"
                            );
                            continue;
                        }
                        Err(err) => {
                            tracing::warn!(
                                latest, ?err, timeout = ?progress_timeout,
                                "🛜 finalized subscription silent and the progress probe failed — reconnecting"
                            );
                            Err(subxt::Error::Other(format!("progress probe failed: {err}")))
                        }
                    },
                } };
                match next {
                    Ok(Some(mut block)) => {
                        let n = block.number() as u64;
                        if n <= latest {
                            // Re-delivery (sub-fork retraction etc.). Skip — we already
                            // yielded this height or older.
                            tracing::debug!(n, latest, "🛜 non-advancing block, skipping");
                            continue;
                        }

                        // Bound the in-memory backfill. The walk below buffers the entire gap in
                        // `backfill` before draining, so a huge (but servable) gap could OOM the
                        // pod mid-recovery. Bail before fetching anything: end the stream like the
                        // permanent-pruned path so a restart re-seeds from the current head with no
                        // backfill (`n > latest` here, so `gap` never underflows).
                        let gap = n - latest - 1;
                        if gap > MAX_BACKFILL_BLOCKS {
                            tracing::error!(
                                n, latest, gap, cap = MAX_BACKFILL_BLOCKS,
                                "🧱 cc3 backfill gap exceeds cap — ending the cc3 stream; \
                                 a restart re-seeds from the current head with no backfill"
                            );
                            return;
                        }

                        // Walk parents until we close the gap to `latest`. Accumulate
                        // (n, events) tuples in `backfill`, then drain in reverse so
                        // downstream sees them in ascending order.
                        //
                        // Retry the head block's events fetch the same way the parent walk does
                        // below: a transient failure (usually a connection drop) must not silently
                        // skip this finalized height. Re-fetch the block by hash on each retry so
                        // we're never bound to a dead connection. Unbounded + backoff, matching the
                        // resubscribe policy — a finalized block's events are always eventually
                        // fetchable once the connection is healthy.
                        let mut head_backoff = RESUBSCRIBE_BACKOFF_START;
                        let head_events = loop {
                            match block.events().await {
                                Ok(e) => break e,
                                Err(err) => {
                                    if is_permanent_backfill_error(&err) {
                                        tracing::error!(
                                            n, ?err,
                                            "🧱 head block state permanently unavailable (pruned) — ending the cc3 stream; \
                                             a restart re-seeds from the current head"
                                        );
                                        return;
                                    }
                                    tracing::warn!(n, ?err, "🛜 events fetch failed for head block — retrying");
                                    let _ = cc3.reconnect().await;
                                    tokio::time::sleep(head_backoff).await;
                                    head_backoff = (head_backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
                                    match cc3.api().blocks().at(block.hash()).await {
                                        Ok(b) => block = b,
                                        Err(e) => tracing::warn!(n, ?e, "🛜 head block re-fetch failed — retrying"),
                                    }
                                }
                            }
                        };
                        let mut walk_n = n;
                        let mut walk_parent = block.header().parent_hash;
                        backfill.push((walk_n, head_events));

                        let mut last_fetch: Option<std::time::Instant> = None;
                        // Per-parent retry backoff. Doubles on each failure starting at 500ms,
                        // capped at the same 30s as the resubscribe loop. Reset on success so a
                        // single bad block doesn't poison the cap for the rest of the walk.
                        const PARENT_BACKOFF_MAX: std::time::Duration = std::time::Duration::from_secs(30);
                        const PARENT_BACKOFF_START: std::time::Duration = std::time::Duration::from_millis(500);
                        while walk_n > latest + 1 {
                            // Throttle: keep at least `backfill_min_interval` between
                            // consecutive parent fetches so a long gap (post-outage recovery)
                            // doesn't burst the cc3 RPC.
                            if let Some(last) = last_fetch {
                                let elapsed = last.elapsed();
                                if elapsed < backfill_min_interval {
                                    tokio::time::sleep(backfill_min_interval - elapsed).await;
                                }
                            }
                            // Retry the parent fetch + events fetch as a single unit. A subxt
                            // `Block` is bound to the connection that produced it, so an
                            // `events()` failure invalidates the `Block` we just got — we
                            // re-fetch both from a fresh connection. Unbounded retry: keeps
                            // partial progress (`backfill` is preserved across attempts) and
                            // matches the resubscribe loop's "ride out the outage" policy.
                            // Cancellation point: dropping the outer stream future drops the
                            // sleep below.
                            let mut backoff = PARENT_BACKOFF_START;
                            let (parent_n, parent_events, next_parent) = loop {
                                match cc3.api().blocks().at(walk_parent).await {
                                    Err(err) => {
                                        if is_permanent_backfill_error(&err) {
                                            tracing::error!(
                                                parent = ?walk_parent, ?err,
                                                "🧱 backfill hit permanently pruned state (outage outlasted the node's pruning \
                                                 horizon) — ending the cc3 stream; a restart re-seeds from the current head"
                                            );
                                            return;
                                        }
                                        tracing::warn!(parent = ?walk_parent, ?err, "🛜 parent fetch failed during backfill — retrying");
                                        let _ = cc3.reconnect().await;
                                        tokio::time::sleep(backoff).await;
                                        backoff = (backoff * 2).min(PARENT_BACKOFF_MAX);
                                    }
                                    Ok(b) => match b.events().await {
                                        Err(err) => {
                                            if is_permanent_backfill_error(&err) {
                                                tracing::error!(
                                                    n = b.number() as u64, ?err,
                                                    "🧱 backfill hit permanently pruned state (outage outlasted the node's pruning \
                                                     horizon) — ending the cc3 stream; a restart re-seeds from the current head"
                                                );
                                                return;
                                            }
                                            tracing::warn!(n = b.number() as u64, ?err, "🛜 parent events fetch failed — retrying");
                                            let _ = cc3.reconnect().await;
                                            tokio::time::sleep(backoff).await;
                                            backoff = (backoff * 2).min(PARENT_BACKOFF_MAX);
                                        }
                                        Ok(events) => {
                                            break (b.number() as u64, events, b.header().parent_hash);
                                        }
                                    },
                                }
                            };
                            last_fetch = Some(std::time::Instant::now());
                            walk_n = parent_n;
                            walk_parent = next_parent;
                            backfill.push((walk_n, parent_events));
                        }

                        if backfill.len() > 1 {
                            tracing::info!(latest, head = n, gap = (n - latest - 1), "🛟 cc3 stream backfill");
                        }

                        // Record progress before yielding: a `yield` parks this generator
                        // until the consumer polls again, so noting afterwards would lag the
                        // consumer's view by one block.
                        latest = n;
                        if let Some(p) = &progress {
                            p.note(n);
                        }
                        for (block_n, events) in backfill.drain(..).rev() {
                            yield StreamEvents::new(
                                block_n as attestor_primitives::Height,
                                events,
                                &chain_keys,
                            );
                        }
                    }
                    Ok(None) | Err(_) => {
                        // Stream ended or errored. Reconnect (which has its own shared
                        // backoff) and re-subscribe. *Unbounded* — RPC downtime can be long
                        // and the right behavior is to ride it out rather than crash.
                        // Cancellation point is the sleep below: shutdown drops this future.
                        if let Err(err) = &next {
                            tracing::warn!(?err, "🛜 cc3 finalized subscription failed");
                        }
                        let mut backoff = RESUBSCRIBE_BACKOFF_START;
                        let new_finalized = loop {
                            tracing::warn!("🛜 cc3 stream lost — reconnecting + re-subscribing");
                            if cc3.reconnect().await.is_err() {
                                tokio::time::sleep(backoff).await;
                                backoff = (backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
                                continue;
                            }
                            match cc3.api().blocks().subscribe_finalized().await {
                                Ok(f) => break f,
                                Err(err) => {
                                    tracing::warn!(?err, "🛜 re-subscribe failed");
                                    tokio::time::sleep(backoff).await;
                                    backoff = (backoff * 2).min(RESUBSCRIBE_BACKOFF_MAX);
                                }
                            }
                        };
                        finalized = new_finalized;
                    }
                }
            }
        }
        .boxed();

        Ok(Self { stream })
    }
}

impl futures::Stream for StreamCC3 {
    type Item = StreamEvents;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;
        self.stream.poll_next_unpin(cx)
    }
}

pub struct StreamEvents {
    stream: std::pin::Pin<
        Box<
            dyn futures::Stream<Item = Result<cc_client::attestation::CcEvent, Error>>
                + Send
                + Sync,
        >,
    >,
    block_number: attestor_primitives::Height,
}

impl std::fmt::Debug for StreamEvents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamEvents")
            .field("block_number", &self.block_number)
            .finish()
    }
}

impl StreamEvents {
    pub fn new(
        block_number: attestor_primitives::Height,
        events: subxt::events::Events<subxt::SubstrateConfig>,
        chain_keys: &[attestor_primitives::ChainKey],
    ) -> Self {
        use futures::TryStreamExt as _;

        // Collect so the boxed stream is `'static` (extract_events borrows `events`).
        let extracted: Vec<_> = cc_client::Client::extract_events(chain_keys, &events).collect();

        let stream = Box::pin(futures::stream::iter(extracted).map_err(Error::Subxt));

        Self {
            block_number,
            stream,
        }
    }

    pub fn block_number(&self) -> attestor_primitives::Height {
        self.block_number
    }
}

impl futures::Stream for StreamEvents {
    type Item = Result<cc_client::attestation::CcEvent, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.stream.as_mut().poll_next(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::is_permanent_backfill_error;

    fn other(msg: &str) -> subxt::Error {
        subxt::Error::Other(msg.to_owned())
    }

    /// Pin the substrate pruned/missing-block message shapes the classifier must recognize —
    /// these are the errors an outage-outlasting-the-pruning-horizon backfill produces, and
    /// missing one silently reverts to today's retry-forever wedge.
    #[test]
    fn recognizes_pruned_state_messages() {
        for msg in [
            "UnknownBlock: State already discarded for 0xabc…",
            "Client error: UnknownBlock: State already discarded for 0xdead",
            "UnknownBlock: Header was not found in the database: 0xabc",
            "unknown block: Body was not found for 0xabc",
        ] {
            assert!(
                is_permanent_backfill_error(&other(msg)),
                "should be permanent: {msg}"
            );
        }
    }

    /// Transient failures must never classify as permanent — that direction would turn an
    /// ordinary outage into a stream death + restart loop.
    #[test]
    fn transient_messages_keep_retrying() {
        for msg in [
            "connection reset by peer",
            "the background task closed",
            "request timed out",
            "RPC error: server is overloaded",
        ] {
            assert!(
                !is_permanent_backfill_error(&other(msg)),
                "should be transient: {msg}"
            );
        }
    }
}
