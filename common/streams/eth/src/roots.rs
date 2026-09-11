use crate::Error;
use user::prelude::*;

#[derive(builder::Builder, Clone)]
pub struct Config {
    pub client: eth::Client,
    pub start_height: attestor_primitives::Height,
    /// How long the `newHeads` subscription may stay silent before the node is probed for its
    /// head. A quiet chain (probe agrees nothing new happened) is left alone; a chain that moved
    /// while the subscription said nothing, or a node that cannot be reached, ends the stream so
    /// the outer layer reconnects. Under an attested bound a silent stream is otherwise
    /// indistinguishable from "nothing new to fetch", which is how a dead socket hid for
    /// 13 minutes in the Sepolia run. 120 s is ten Ethereum slots and forty BSC blocks.
    #[default(std::time::Duration::from_secs(120))]
    pub head_silence_timeout: std::time::Duration,

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

    /// How often to poll `eth_blockNumber` alongside the `newHeads` subscription. The poll is
    /// the liveness floor: a subscription that acknowledges but stops delivering headers (seen
    /// through proxies and load balancers) no longer stalls the stream, and no work waits on
    /// a future header before it can start.
    #[default(DEFAULT_HEAD_POLL_INTERVAL)]
    pub head_poll_interval: std::time::Duration,

    /// Deadline for the individual RPC calls made while establishing the stream (subscribe,
    /// initial head). alloy transports have no default timeout, so without this a blackholed
    /// endpoint could hang construction indefinitely.
    #[default(DEFAULT_RPC_CALL_TIMEOUT)]
    pub rpc_call_timeout: std::time::Duration,
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

pub const DEFAULT_HEAD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(12);
pub const DEFAULT_RPC_CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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

    // Initial subscribe + head read, repairing the client between attempts. This runs from
    // `new()` and from `reset()` (whose backup config's client may have died since
    // construction), and as the second layer under `StreamRoots::reconnect`. `eth::Client` is
    // a value clone — a dead connection inside it never self-heals — so a loop that only
    // re-`subscribe()`s can spin on a dead client forever, pinning production until the
    // watchdog restarts the pod. The repaired client stays in `config`, so the block fetches
    // in the stream body use it too. First attempt goes straight to `subscribe()` so a healthy
    // construction pays no extra dial.
    //
    // The starting head comes from `eth_blockNumber`, not from the first `newHeads` frame:
    // catch-up begins immediately, and a subscription that acknowledges but never delivers
    // (or a chain that is simply idle) cannot hold construction hostage. Each attempt is
    // bounded by `rpc_call_timeout`.
    let mut delays = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
        .max_delay(std::time::Duration::from_millis(5_000))
        .map(tokio_retry::strategy::jitter);
    let (stream_headers, head) = loop {
        let attempt = async {
            let headers = config.client.subscribe().await.map_err(Error::Client)?;
            let head = config
                .client
                .get_last_block()
                .await
                .map_err(Error::Client)?;
            Ok::<_, Error>((headers, head))
        };
        match tokio::time::timeout(config.rpc_call_timeout, attempt).await {
            Ok(Ok(seed)) => break seed,
            Ok(Err(err)) => {
                tracing::warn!(
                    ?err,
                    "Eth subscribe/head read failed — repairing client and retrying"
                )
            }
            Err(_) => tracing::warn!(
                timeout = ?config.rpc_call_timeout,
                "Eth subscribe/head read timed out — repairing client and retrying"
            ),
        }
        if let Err(err) = config.client.reconnect().await {
            tracing::warn!(?err, "Eth client reconnect failed");
        }
        let delay = delays
            .next()
            .unwrap_or(std::time::Duration::from_millis(5_000));
        tokio::time::sleep(delay).await;
    };

    // Head numbers from the subscription, merged with a periodic `eth_blockNumber` poll. The
    // merged stream ends when the subscription ends (that is how a dead socket surfaces and
    // triggers reconnection); poll results only ever advance the target.
    let subscribed = stream_headers.map(|header| Some(header.number));
    let poll_client = config.client.clone();
    let poll_timeout = config.rpc_call_timeout;
    let polled = futures::stream::unfold(
        tokio::time::interval(config.head_poll_interval),
        |mut ticker| async move {
            ticker.tick().await;
            Some(((), ticker))
        },
    )
    .skip(1) // the first tick fires immediately; the seed above already covered it
    .then(move |_| {
        let client = poll_client.clone();
        async move {
            match tokio::time::timeout(poll_timeout, client.get_last_block()).await {
                Ok(Ok(n)) => Some(n),
                Ok(Err(err)) => {
                    tracing::debug!(%err, "head poll failed; relying on subscription");
                    None
                }
                Err(_) => {
                    tracing::debug!("head poll timed out; relying on subscription");
                    None
                }
            }
        }
    })
    .filter_map(futures::future::ready);

    // Bound pipeline. Every source head — the first one above and each one the subscription
    // delivers — becomes a candidate upper bound through `config.bound`, and the block numbers
    // between the last fetched height and that bound are what this stream fetches next
    // (`heights_to_fetch`). With a fixed lag that is the classic `head - lag` walk, one block per
    // head, gaps backfilled; with a block tag or an attested bound the bound moves in jumps and
    // a whole range is released at once. A bound that cannot be resolved for a head is skipped:
    // the next head retries, and the walk guarantees no block is skipped or fetched twice.
    // The seed head was read above via `eth_blockNumber`, so it is handed to the watchdog as
    // its baseline: a subscription that dies right after it must still be caught. The watchdog
    // wraps the merged subscription + poll stream: while polls keep flowing it never fires, and
    // a dead socket still ends the merged stream and so reconnects.
    let heads = futures::stream::once(futures::future::ready(head)).chain(end_on_silence(
        merge_heads(subscribed, polled.map(Some)).boxed(),
        config.client.clone(),
        config.head_silence_timeout,
        Some(head),
    ));
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
            // Each head and each change of the published bound is a candidate, so an
            // attestation landing between heads is acted on without waiting. The bound is
            // clamped to the newest head *this* subscription has delivered: the attestors may be
            // ahead of the node we read from, and a released height above that node's head fails
            // its fetch, which the outer layer treats as a dead connection and tears the whole
            // batch down. Same invariant `Maturity::mature_height` keeps for block tags. The
            // stream ends when the heads end, so a silent subscription still reconnects.
            enum Obs {
                Head(u64),
                Bound(Option<u64>),
                End,
            }
            let on_heads = heads
                .map(Obs::Head)
                .chain(futures::stream::once(futures::future::ready(Obs::End)));
            let on_bounds = watch_values(rx).map(Obs::Bound);
            futures::stream::select(on_heads, on_bounds)
                .take_while(|obs| futures::future::ready(!matches!(obs, Obs::End)))
                .scan((None, None), |(head, bound), obs| {
                    match obs {
                        // High-water mark: heads arrive from the subscription and the poll
                        // interleaved, so a lagging poll can land after a newer head and must
                        // not pull the clamp back below what this node has already shown us.
                        Obs::Head(h) => *head = newest_head(*head, h),
                        Obs::Bound(b) => *bound = b,
                        Obs::End => unreachable!("filtered by take_while"),
                    }
                    futures::future::ready(Some(clamp_to_head(*bound, *head)))
                })
                .boxed()
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

/// An externally published bound, limited to the newest head this subscription has seen.
/// `None` until both are known: a bound with no head to clamp against must not release anything.
fn clamp_to_head(bound: Option<u64>, head: Option<u64>) -> Option<u64> {
    Some(bound?.min(head?))
}

/// The newest head seen so far. Heads reach the attested pipeline from two sources, the
/// `newHeads` subscription and the periodic `eth_blockNumber` poll, and they interleave, so an
/// observation is only allowed to move the mark forward: a poll answered before a newer head
/// arrived must not lower the clamp and withhold heights that are already fetchable.
fn newest_head(seen: Option<u64>, observed: u64) -> Option<u64> {
    Some(seen.map_or(observed, |s| s.max(observed)))
}

/// What a silent `newHeads` subscription means once the node has been asked for its head.
#[derive(Debug, PartialEq, Eq)]
enum Silence {
    /// The node agrees nothing new has happened; keep waiting.
    QuietChain,
    /// The chain advanced (or the node is unreachable) while the subscription said nothing.
    DeadSubscription,
}

/// `baseline` is the newest head this stream knows about: the last one the subscription
/// delivered, or, when it has delivered nothing yet, the last probe. Returns the verdict and the
/// baseline to carry forward, so a subscription that is silent from the start is judged against
/// its own first probe on the next round rather than never.
fn judge_silence(baseline: Option<u64>, probe: Result<u64, String>) -> (Silence, Option<u64>) {
    match (baseline, probe) {
        (None, Ok(head)) => (Silence::QuietChain, Some(head)),
        (Some(seen), Ok(head)) if head <= seen => (Silence::QuietChain, Some(seen)),
        (_, Ok(head)) => (Silence::DeadSubscription, Some(head)),
        (seen, Err(_)) => (Silence::DeadSubscription, seen),
    }
}

/// Ends a head-number stream when it stays silent for `timeout` and the node, asked directly,
/// says the chain has moved on or cannot be reached. A chain that genuinely produced no block
/// (dev chains mining on demand, a stalled rollup) keeps the stream open. Ending the stream is
/// what makes the outer layer reconnect, so a dead socket surfaces as a reconnect instead of
/// as an endless "nothing new to fetch".
pub(crate) fn end_on_silence(
    heads: stream_util::BoxedStream<u64>,
    client: eth::Client,
    timeout: std::time::Duration,
    baseline: Option<u64>,
) -> impl futures::Stream<Item = u64> {
    use futures::StreamExt as _;
    futures::stream::unfold(
        (heads, client, baseline),
        move |(mut heads, client, mut baseline)| async move {
            loop {
                match tokio::time::timeout(timeout, heads.next()).await {
                    Ok(Some(head)) => return Some((head, (heads, client, Some(head)))),
                    Ok(None) => return None,
                    Err(_) => {
                        let probe =
                            match tokio::time::timeout(timeout, client.get_last_block()).await {
                                Ok(Ok(head)) => Ok(head),
                                Ok(Err(err)) => Err(err.to_string()),
                                Err(_) => Err("head probe timed out".to_owned()),
                            };
                        let (verdict, next_baseline) = judge_silence(baseline, probe.clone());
                        match verdict {
                            Silence::QuietChain => {
                                tracing::debug!(
                                    ?baseline,
                                    ?probe,
                                    silent_for = ?timeout,
                                    "no new heads, and the node agrees the chain is quiet"
                                );
                                baseline = next_baseline;
                            }
                            Silence::DeadSubscription => {
                                tracing::warn!(
                                    ?baseline,
                                    ?probe,
                                    silent_for = ?timeout,
                                    "newHeads subscription went silent while the chain moved on; \
                                     ending the stream so it reconnects"
                                );
                                return None;
                            }
                        }
                    }
                }
            }
        },
    )
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

/// Merge subscription head numbers with polled head numbers. `subscribed` yields `Some(n)` per
/// header and must be followed by a `None` sentinel when the subscription ends; the merged
/// stream ends there, so a dead socket still surfaces as "stream ended" to the caller. Polled
/// values are plain `Some(n)`.
fn merge_heads<S, P>(subscribed: S, polled: P) -> impl futures::Stream<Item = u64>
where
    S: futures::Stream<Item = Option<u64>>,
    P: futures::Stream<Item = Option<u64>>,
{
    use futures::StreamExt as _;
    let subscribed = subscribed.chain(futures::stream::once(futures::future::ready(None)));
    futures::stream::select(subscribed, polled)
        .take_while(|head| futures::future::ready(head.is_some()))
        .filter_map(futures::future::ready)
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

    #[test]
    fn a_lagging_poll_never_lowers_the_observed_head() {
        assert_eq!(newest_head(None, 100), Some(100));
        assert_eq!(
            newest_head(Some(100), 105),
            Some(105),
            "newer head advances"
        );
        assert_eq!(
            newest_head(Some(105), 100),
            Some(105),
            "a stale poll answered late keeps the high-water mark"
        );
        assert_eq!(
            clamp_to_head(Some(104), newest_head(Some(105), 100)),
            Some(104),
            "so a bound already fetchable is not withheld by it"
        );
    }

    #[test]
    fn the_attested_bound_never_exceeds_the_observed_head() {
        assert_eq!(clamp_to_head(None, None), None);
        assert_eq!(
            clamp_to_head(Some(100), None),
            None,
            "no head yet: release nothing"
        );
        assert_eq!(clamp_to_head(None, Some(100)), None);
        assert_eq!(clamp_to_head(Some(100), Some(150)), Some(100));
        assert_eq!(
            clamp_to_head(Some(150), Some(100)),
            Some(100),
            "lagging node: clamp"
        );
    }

    #[test]
    fn silence_is_only_fatal_when_the_chain_moved_or_the_node_is_gone() {
        assert_eq!(
            judge_silence(Some(100), Ok(100)),
            (Silence::QuietChain, Some(100))
        );
        assert_eq!(
            judge_silence(Some(100), Ok(99)),
            (Silence::QuietChain, Some(100))
        );
        assert_eq!(
            judge_silence(Some(100), Ok(101)),
            (Silence::DeadSubscription, Some(101))
        );
        assert_eq!(
            judge_silence(Some(100), Err("connection refused".into())),
            (Silence::DeadSubscription, Some(100))
        );
    }

    /// A subscription silent from the very start has no head to compare against. The first
    /// probe becomes the baseline, so the *second* silent round catches a chain that moved.
    #[test]
    fn a_subscription_silent_from_the_start_is_judged_against_its_first_probe() {
        let (verdict, baseline) = judge_silence(None, Ok(5));
        assert_eq!((verdict, baseline), (Silence::QuietChain, Some(5)));
        assert_eq!(
            judge_silence(baseline, Ok(5)),
            (Silence::QuietChain, Some(5)),
            "still nothing happened"
        );
        assert_eq!(
            judge_silence(baseline, Ok(6)),
            (Silence::DeadSubscription, Some(6)),
            "the chain moved and the subscription said nothing"
        );
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

    #[tokio::test]
    async fn merged_heads_keep_flowing_from_polls_while_the_subscription_is_silent() {
        // Subscription acknowledged but never delivers; polls carry the stream.
        let silent = futures::stream::pending::<Option<u64>>();
        let polls = futures::stream::iter(vec![Some(4u64), Some(6)]);
        let got: Vec<u64> = merge_heads(silent, polls).take(2).collect().await;
        assert_eq!(got, vec![4, 6]);
    }

    #[tokio::test]
    async fn merged_heads_end_when_the_subscription_ends() {
        let subscribed = futures::stream::iter(vec![Some(1u64), Some(2)]);
        let polls = futures::stream::pending::<Option<u64>>();
        let got: Vec<u64> = merge_heads(subscribed, polls).collect().await;
        assert_eq!(got, vec![1, 2]);
    }
}
