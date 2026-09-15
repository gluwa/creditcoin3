use crate::Error;
use user::prelude::*;

#[derive(builder::Builder, Clone)]
pub struct Config {
    pub client: eth::Client,
    pub start_height: attestor_primitives::Height,
    /// Where the upper bound of fetchable heights comes from. See [`Boundary`].
    ///
    /// Named `bound`, not `boundary`: the builder derive names each typestate parameter after
    /// its field, and a `Boundary` parameter would shadow the enum inside the generated setter.
    pub bound: Boundary,

    /// Maximum number of concurrent block fetch tasks (IO-bound).
    pub max_concurrency: std::num::NonZeroUsize,

    /// Maximum number of parallel block root merkleization (CPU-bound).
    pub max_parallelism: std::num::NonZeroUsize,

    /// Source-chain block encoding. Derived from CC3 supported-chain metadata by
    /// callers that know the chain; defaults to V1 (the only encoding today) so
    /// existing builders keep working.
    #[default(usc_abi_encoding::common::EncodingVersion::V1)]
    pub encoding: usc_abi_encoding::common::EncodingVersion,
}

/// The upper bound of the heights this stream fetches, and where it comes from.
///
/// Every source head the subscription delivers is turned into a candidate upper bound; the
/// heights between the last one fetched and that bound are what the stream fetches next. Only
/// how the bound is derived differs.
#[derive(Clone, Debug)]
pub enum Boundary {
    /// Resolve maturity against the source node itself, per head: a fixed lag behind it or the
    /// node's `safe` / `finalized` tag. This is what an attestor runs, because attestors *are*
    /// the maturity decision.
    Source(eth::Maturity),
    /// Follow a bound published by something else, as a high-water mark that only ever grows.
    /// The archiver runs this with the latest attested height for its chain key, so it can
    /// never hold a root the attestors have not agreed on and never needs a maturity policy of
    /// its own. `None` means no bound has been published yet; nothing is fetched until one is.
    /// Besides each source head, every change of the value is a candidate bound in its own
    /// right, so a bound advancing between heads is acted on immediately.
    Attested(tokio::sync::watch::Receiver<Option<u64>>),
}

impl std::fmt::Display for Boundary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(maturity) => write!(f, "source-resolved {maturity}"),
            Self::Attested(rx) => match *rx.borrow() {
                Some(height) => write!(f, "attested height (currently {height})"),
                None => write!(f, "attested height (none published yet)"),
            },
        }
    }
}

/// Ordered Eth root stream, backed by [`eth::Client`] under the hood.
///
/// This stream is optimized to make fast concurrent progress in chain tip polling, block fetching
/// and merkleization. Performance characteristics and backpressure can be tweaked via
/// [`Config::max_concurrency`] and [`Config::max_parallelism`].
///
/// It is generally beneficial for the async runtime being used to configure a number of worker
/// threads greater than or equal to the max concurrency + 1 for optimal throughput.
///
/// Implements capped exponential retry without unbounded attempts in order to handle RPC
/// disconnections. This stream can be considered infinite and will never return [`None`].
pub struct StreamRoots {
    stream: sync_wrapper::SyncStream<stream_util::BoxedStream<stream_util::RootInfo>>,
    config: Config,
}

impl StreamRoots {
    pub async fn new(mut config: Config) -> Self {
        use futures::StreamExt as _;

        let start_height = config.start_height;
        let max_parallelism = config.max_parallelism.get();
        let mut stream_blocks = stream_rpc(config.clone()).await;

        let mut next = start_height;
        let mut roots = tokio::task::JoinSet::<stream_util::RootInfo>::new();
        let mut heap = std::collections::BinaryHeap::with_capacity(max_parallelism);

        let backup = config.clone();

        let stream = async_stream::stream! {
            loop {
                tokio::select! {
                    // TASK 1] Poll source chain blocks
                    block = stream_blocks.next() => {
                        match block {
                            Some(Ok(block)) => {
                                // Backpressure: limit the number of blocks being processed
                                // in parallel to `max_parallelism`
                                while roots.len() >= max_parallelism {

                                    // Spawn the root computation anyways if the block stream has
                                    // ended. That way we can drain existing roots before exiting.
                                    let Some(root) = roots.join_next().await else {
                                        break;
                                    };

                                    // Tries to drain existing roots to make space for new
                                    // computations.
                                    match root {
                                        Ok(info) => {

                                            // Since blocks roots are computed in parallel,
                                            // they need to be re-ordered manually
                                            heap.push(std::cmp::Reverse(info));
                                            while heap
                                                .peek()
                                                .is_some_and(|info| info.0.height == next)
                                            {
                                                next += 1;
                                                yield heap
                                                    .pop()
                                                    .expect("Checked above")
                                                    .0;
                                            }
                                        },
                                        Err(err) => {
                                            if err.is_panic() {
                                                std::panic::resume_unwind(err.into_panic());
                                            }
                                        },
                                    }
                                }

                                // Actual root computation. No more than `max_parallelism` roots
                                // may be computed at once.
                                roots.spawn_blocking(move || {
                                    stream_util::RootInfo {
                                        height: block.number(),
                                        root: eth::simple_merkle_tree(&block).root(),
                                        hash: attestor_primitives::Digest::from(*block.hash()),
                                    }
                                });
                            },
                            Some(Err(err)) if err.is_shutdown() => {
                                // User-initiated shutdown (Ctrl+C / service stop) propagated up
                                // from the inner block stream. This is NOT a connection failure:
                                // drain in-flight root computations and terminate the outer
                                // stream cleanly instead of reconnecting.
                                tracing::info!("Eth root stream shutting down on user interrupt");
                                roots.abort_all();
                                while !roots.is_empty() {
                                    if let Some(Err(err)) = roots.join_next().await {
                                        if err.is_panic() {
                                            std::panic::resume_unwind(err.into_panic());
                                        }
                                    }
                                }
                                return;
                            },
                            Some(Err(err)) => {
                                // Failed to retrieve source chain block, try and regenerate the
                                // stream (this can only be an RPC error)
                                tracing::error!(%err, "Eth connection error");
                                heap.clear();

                                // Removes pending root calls
                                roots.abort_all();
                                while !roots.is_empty() {
                                    if let Some(Err(err)) = roots.join_next().await {
                                        if err.is_panic() {
                                            std::panic::resume_unwind(err.into_panic());
                                        }
                                    }
                                }

                                let (client, stream) = Self::reconnect(&config, next).await;

                                config.client = client;
                                stream_blocks = stream;
                            },
                            None => {
                                // Eth block stream should never end. If it does this indicates an
                                // RPC error in which case we need to reconnect.
                                tracing::error!("Eth connection lost");
                                heap.clear();

                                // Removes pending root calls
                                roots.abort_all();
                                while !roots.is_empty() {
                                    if let Some(Err(err)) = roots.join_next().await {
                                        if err.is_panic() {
                                            std::panic::resume_unwind(err.into_panic());
                                        }
                                    }
                                }

                                let (client, stream) = Self::reconnect(&config, next).await;

                                config.client = client;
                                stream_blocks = stream;
                            }
                        }
                    }
                    // TASK 2] Drain completed block roots
                    Some(root) = roots.join_next(), if !roots.is_empty() => {
                        match root {
                            Ok(info) => {
                                heap.push(std::cmp::Reverse(info));

                                // Drain as many roots as possible to deal with sporadic bursts in
                                // ordering.
                                while heap
                                    .peek()
                                    .is_some_and(|info| info.0.height == next)
                                {
                                    next += 1;
                                    yield heap.pop().expect("Checked above").0;
                                }
                            },
                            Err(err) => {
                                if err.is_panic() {
                                    std::panic::resume_unwind(err.into_panic())
                                }
                            }
                        }
                    }
                }
            }
        }
        .boxed();

        Self {
            stream: sync_wrapper::SyncStream::new(stream),
            config: backup,
        }
    }

    async fn reconnect(
        config: &Config,
        next: attestor_primitives::Height,
    ) -> (
        eth::Client,
        stream_util::BoxedStream<Result<eth::OrderedBlock, Error>>,
    ) {
        let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
            .max_delay(std::time::Duration::from_millis(5_000))
            .map(tokio_retry::strategy::jitter);

        let reconnect = || {
            tracing::warn!("Reconnecting to Eth...");

            let mut config = config.clone();
            config.start_height = next;

            async move {
                config.client.reconnect().await.map_err(Error::Client)?;
                let stream = stream_rpc(config.clone()).await;

                Ok::<_, Error>((config.client, stream))
            }
        };

        let retry = tokio_retry::Retry::spawn(strategy, reconnect);
        retry.await.expect("Unbounded retry cannot error")
    }
}

impl futures::Stream for StreamRoots {
    type Item = stream_util::RootInfo;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;
        self.stream.poll_next_unpin(cx)
    }
}

impl stream_util::ChainData<stream_util::RootInfo> for StreamRoots {
    async fn reset(&self, info: stream_util::AttestationInfo) -> Self {
        let mut config = self.config.clone();
        config.start_height = info.height;

        Self::new(config).await
    }
}

async fn stream_rpc(
    mut config: Config,
) -> stream_util::BoxedStream<Result<eth::OrderedBlock, Error>> {
    use futures::StreamExt as _;

    // Initial subscribe, repairing the client between attempts. This runs from `new()` and from
    // `reset()` (whose backup config's client may have died since construction), and as the
    // second layer under `StreamRoots::reconnect`. `eth::Client` is a value clone — a dead
    // connection inside it never self-heals — so a loop that only re-`subscribe()`s can spin on
    // a dead client forever, pinning production until the watchdog restarts the pod. The
    // repaired client stays in `config`, so the block fetches in the stream body use it too.
    // First attempt goes straight to `subscribe()` so a healthy construction pays no extra dial.
    let mut delays = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
        .max_delay(std::time::Duration::from_millis(5_000))
        .map(tokio_retry::strategy::jitter);
    let (stream_headers, next) = loop {
        match config.client.subscribe().await.map_err(Error::Client) {
            Ok(mut stream_headers) => match stream_headers.next().await {
                Some(header) => break (stream_headers, header.number),
                None => tracing::warn!("Eth header stream ended before yielding — retrying"),
            },
            Err(err) => {
                tracing::warn!(?err, "Eth subscribe failed — repairing client and retrying");
            }
        }
        if let Err(err) = config.client.reconnect().await {
            tracing::warn!(?err, "Eth client reconnect failed");
        }
        let delay = delays
            .next()
            .unwrap_or(std::time::Duration::from_millis(5_000));
        tokio::time::sleep(delay).await;
    };

    // Bound pipeline. Every source head — the first one above and each one the subscription
    // delivers — becomes a candidate upper bound through `config.bound`, and the block numbers
    // between the last fetched height and that bound are what this stream fetches next
    // (`heights_to_fetch`). With a fixed lag that is the classic `head - lag` walk, one block per
    // head, gaps backfilled; with a block tag or an attested bound the bound moves in jumps and
    // a whole range is released at once. A bound that cannot be resolved for a head is skipped:
    // the next head retries, and the walk guarantees no block is skipped or fetched twice.
    let heads = futures::stream::once(futures::future::ready(next))
        .chain(stream_headers.map(|header| header.number));
    let bounds: stream_util::BoxedStream<Option<u64>> = match config.bound.clone() {
        Boundary::Source(maturity) => {
            let client = config.client.clone();
            heads
                .then(move |head| {
                    let client = client.clone();
                    async move { (head, maturity.mature_height(&client, head).await) }
                })
                .map(move |(head, mature)| match mature {
                    Ok(mature) => mature,
                    Err(err) => {
                        tracing::warn!(
                            head,
                            %maturity,
                            %err,
                            "could not resolve mature height for this head; retrying on the next one"
                        );
                        None
                    }
                })
                .boxed()
        }
        Boundary::Attested(rx) => {
            // Each head re-reads the published bound; each change of the bound is a candidate
            // too, so an attestation landing between heads is acted on without waiting.
            let on_heads = {
                let rx = rx.clone();
                heads.map(move |_| *rx.borrow())
            };
            futures::stream::select(on_heads, watch_values(rx)).boxed()
        }
    };
    let mut stream_n = heights_to_fetch(config.start_height, bounds).boxed();

    let mut blocks = tokio::task::JoinSet::new();

    async_stream::stream! {
        loop {
            tokio::select! {
                // TASK 1] Poll source chain headers
                Some(n) = stream_n.next() => {

                    // Backpressure: limit the number of blocks which can be fetched
                    // concurrently to `max_concurrency`.
                    while blocks.len() >= config.max_concurrency.get() {

                        // Tries to drain existing blocks to make space for new ones.
                        if let Some(block) = blocks.join_next().await {
                            match block
                            {
                                Ok(Ok(block)) => yield Ok(block),
                                Ok(Err(Interrupt::Cont(err))) => yield Err(err),
                                // User-initiated shutdown: surface a typed `Shutdown` error and
                                // terminate the stream. The outer `StreamRoots` layer recognizes
                                // this and exits cleanly instead of treating the stream end as a
                                // connection failure and reconnecting.
                                Ok(Err(Interrupt::Stop)) => {
                                    yield Err(Error::Shutdown);
                                    return;
                                }
                                Err(err) => {
                                    if err.is_panic() {
                                        std::panic::resume_unwind(err.into_panic());
                                    }
                                }
                            }
                        }
                    }

                    let eth = config.client.clone();
                    let encoding = config.encoding;

                    // Actual block fetching. No more than `max_concurrency` blocks may be
                    // fetched at once. `n` is already a mature height.
                    blocks.spawn(async move {
                        eth.get_block(
                            n,
                            encoding
                        )
                        .await
                        .map_interrupt(Error::Client)
                    });
                }
                // TASK 2] Drain fetched blocks (out-of-order)
                Some(block) = blocks.join_next(), if !blocks.is_empty() => {
                    match block
                    {
                        Ok(Ok(block)) => yield Ok(block),
                        Ok(Err(Interrupt::Cont(err))) => yield Err(err),
                        // User-initiated shutdown: surface a typed `Shutdown` error and terminate
                        // the stream so the outer layer exits cleanly rather than reconnecting.
                        Ok(Err(Interrupt::Stop)) => {
                            yield Err(Error::Shutdown);
                            return;
                        }
                        Err(err) => {
                            if err.is_panic() {
                                std::panic::resume_unwind(err.into_panic());
                            }
                        }
                    }
                }
                else => {
                    // Eth stream has terminated and all block fetching tasks have completed, it is
                    // safe to exit the stream now. This should not happen under normal
                    // circumstances as the eth stream is supposed to be infinite and should be
                    // treated by callers as a connection failure.
                    break;
                }
            }
        }
    }
    .boxed()
}

/// The heights to fetch, in order, as candidate upper `bounds` arrive.
///
/// A bound of `None` (nothing resolvable yet) or one that does not advance past what has already
/// been released contributes nothing, so heads, tag lookups and attested-height changes can all
/// feed this freely and out of step with each other. Every height from `start` upwards is
/// released exactly once.
fn heights_to_fetch<B>(start: u64, bounds: B) -> impl futures::Stream<Item = u64>
where
    B: futures::Stream<Item = Option<u64>>,
{
    use futures::StreamExt as _;
    bounds
        .filter_map(futures::future::ready)
        .scan(start, |next_unfetched, bound| {
            let range = eth::Maturity::newly_mature(*next_unfetched, bound);
            if !range.is_empty() {
                *next_unfetched = bound + 1;
            }
            futures::future::ready(Some(futures::stream::iter(range)))
        })
        .flatten()
}

/// The current value of a watch channel, then every subsequent change, until the sender is gone.
fn watch_values<T: Copy + Send + Sync + 'static>(
    rx: tokio::sync::watch::Receiver<T>,
) -> impl futures::Stream<Item = T> {
    futures::stream::unfold((rx, true), |(mut rx, first)| async move {
        if !first && rx.changed().await.is_err() {
            return None;
        }
        let value = *rx.borrow_and_update();
        Some((value, (rx, false)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;

    #[tokio::test]
    async fn heights_to_fetch_releases_each_height_once_and_ignores_non_advancing_bounds() {
        let bounds = futures::stream::iter([None, Some(5), Some(5), Some(3), None, Some(8)]);
        let got: Vec<u64> = heights_to_fetch(3, bounds).collect().await;
        assert_eq!(got, vec![3, 4, 5, 6, 7, 8]);
    }

    #[tokio::test]
    async fn heights_to_fetch_releases_nothing_below_start() {
        let bounds = futures::stream::iter([Some(2), Some(1)]);
        let got: Vec<u64> = heights_to_fetch(3, bounds).collect().await;
        assert!(got.is_empty());
    }

    /// An attested bound advancing releases the newly covered range without waiting for a source
    /// head, and the initial `None` holds everything back.
    #[tokio::test]
    async fn attested_bound_changes_drive_the_walk_on_their_own() {
        let (tx, rx) = tokio::sync::watch::channel(None);
        let mut heights = Box::pin(heights_to_fetch(10, watch_values(rx)));

        let nothing_yet =
            tokio::time::timeout(std::time::Duration::from_millis(50), heights.next()).await;
        assert!(
            nothing_yet.is_err(),
            "no bound published, nothing may be fetched"
        );

        tx.send(Some(12)).unwrap();
        let mut got = vec![];
        for _ in 0..3 {
            got.push(heights.next().await.unwrap());
        }
        assert_eq!(got, vec![10, 11, 12]);

        tx.send(Some(11)).unwrap(); // a bound moving backwards releases nothing
        tx.send(Some(13)).unwrap();
        assert_eq!(heights.next().await, Some(13));

        drop(tx);
        assert_eq!(heights.next().await, None, "sender gone, stream ends");
    }
}
