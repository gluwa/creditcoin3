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
//! re-implement retry timing. We do bound subscription-retry attempts to avoid an unbounded
//! spin if the node is permanently down.

mod error;
pub use error::Error;

const MAX_RESUBSCRIBE_ATTEMPTS: usize = 6;

#[derive(Debug, builder::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
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

                        let mut walk_failed = false;
                        while walk_n > latest + 1 {
                            let parent = match cc3.api().blocks().at(walk_parent).await {
                                Ok(b) => b,
                                Err(err) => {
                                    tracing::warn!(parent = ?walk_parent, ?err, "🛜 parent fetch failed during backfill");
                                    walk_failed = true;
                                    break;
                                }
                            };
                            let parent_events = match parent.events().await {
                                Ok(e) => e,
                                Err(err) => {
                                    tracing::warn!(n = parent.number() as u64, ?err, "🛜 parent events fetch failed");
                                    walk_failed = true;
                                    break;
                                }
                            };
                            walk_n = parent.number() as u64;
                            walk_parent = parent.header().parent_hash;
                            backfill.push((walk_n, parent_events));
                        }

                        if walk_failed {
                            backfill.clear();
                            continue;
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
                        // backoff) and re-subscribe. Bounded attempts so a permanently-down
                        // node eventually surfaces a crash instead of silent spinning.
                        let mut new_finalized = None;
                        for attempt in 0..MAX_RESUBSCRIBE_ATTEMPTS {
                            tracing::warn!(attempt, "🛜 cc3 stream lost — reconnecting + re-subscribing");
                            if cc3.reconnect().await.is_err() {
                                continue;
                            }
                            match cc3.api().blocks().subscribe_finalized().await {
                                Ok(f) => { new_finalized = Some(f); break; }
                                Err(err) => tracing::warn!(?err, "🛜 re-subscribe failed"),
                            }
                        }
                        match new_finalized {
                            Some(f) => { finalized = f; }
                            None => {
                                tracing::error!("🛜 cc3 stream re-subscribe exhausted retries; closing stream");
                                return;
                            }
                        }
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
