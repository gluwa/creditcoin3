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

#[derive(Debug, builder::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
    /// Minimum interval between consecutive parent-block fetches during backfill — applies only
    /// to gap recovery, not to the live `subscribe_finalized` flow.
    #[default(DEFAULT_BACKFILL_MIN_INTERVAL)]
    backfill_min_interval: std::time::Duration,
}

pub struct StreamCC3 {
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = StreamEvents> + Send>>,
}

impl StreamCC3 {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        let chain_key = config.chain_key;
        let cc3 = config.cc3;
        let backfill_min_interval = config.backfill_min_interval;

        let mut finalized = cc3
            .api()
            .blocks()
            .subscribe_finalized()
            .await
            .map_err(Error::Subxt)?;

        // Seed `latest` from the first block. The stream macro takes over from there.
        let first = finalized
            .try_next()
            .await
            .map_err(Error::Subxt)?
            .ok_or(Error::EndOfStream)?;
        let mut latest = first.number() as u64;
        let first_events = first.events().await.map_err(Error::Subxt)?;

        let stream = async_stream::stream! {
            yield StreamEvents::new(latest as attestor_primitives::Height, first_events, chain_key);

            let mut finalized = finalized;
            // Reusable scratch buffer for the parent-walk backfill. Capacity tuned for
            // typical disconnects of <16 blocks; grows if needed.
            let mut backfill: Vec<(u64, subxt::events::Events<subxt::SubstrateConfig>)> =
                Vec::with_capacity(16);

            loop {
                match finalized.try_next().await {
                    Ok(Some(block)) => {
                        let n = block.number() as u64;
                        if n <= latest {
                            // Re-delivery (sub-fork retraction etc.). Skip — we already
                            // yielded this height or older.
                            tracing::debug!(n, latest, "🛜 non-advancing block, skipping");
                            continue;
                        }

                        // Walk parents until we close the gap to `latest`. Accumulate
                        // (n, events) tuples in `backfill`, then drain in reverse so
                        // downstream sees them in ascending order.
                        let head_events = match block.events().await {
                            Ok(e) => e,
                            Err(err) => {
                                tracing::warn!(n, ?err, "🛜 events fetch failed for head block");
                                continue;
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
                                        tracing::warn!(parent = ?walk_parent, ?err, "🛜 parent fetch failed during backfill — retrying");
                                        let _ = cc3.reconnect().await;
                                        tokio::time::sleep(backoff).await;
                                        backoff = (backoff * 2).min(PARENT_BACKOFF_MAX);
                                    }
                                    Ok(b) => match b.events().await {
                                        Err(err) => {
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

                        for (block_n, events) in backfill.drain(..).rev() {
                            yield StreamEvents::new(
                                block_n as attestor_primitives::Height,
                                events,
                                chain_key,
                            );
                        }
                        latest = n;
                    }
                    Ok(None) | Err(_) => {
                        // Stream ended or errored. Reconnect (which has its own shared
                        // backoff) and re-subscribe. *Unbounded* — RPC downtime can be long
                        // and the right behavior is to ride it out rather than crash.
                        // Cancellation point is the sleep below: shutdown drops this future.
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
        chain_key: attestor_primitives::ChainKey,
    ) -> Self {
        use futures::TryStreamExt as _;

        // Collect so the boxed stream is `'static` (extract_events borrows `events`).
        let extracted: Vec<_> =
            cc_client::Client::extract_events(std::slice::from_ref(&chain_key), &events).collect();

        let stream =
            Box::pin(futures::stream::iter(extracted).map_err(|err| Error::Subxt(err.into())));

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
