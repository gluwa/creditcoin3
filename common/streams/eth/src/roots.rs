use crate::Error;
use user::prelude::*;

#[derive(builder::Builder, Clone)]
pub struct Config {
    pub client: eth::Client,
    pub start_height: attestor_primitives::Height,
    pub finalization_lag: attestor_primitives::Height,

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
    // `Delay` rather than tokio's default `Burst`: this stream is not polled while
    // `expand_heads` drains the seeded `start..=head` range, and after a long catch-up the
    // missed ticks would otherwise fire back-to-back as a flood of `eth_blockNumber` calls on
    // the same socket that carries block fetches and `newHeads`.
    let mut ticker = tokio::time::interval(config.head_poll_interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let polled = futures::stream::unfold(ticker, |mut ticker| async move {
        ticker.tick().await;
        Some(((), ticker))
    })
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
    let heads = merge_heads(subscribed, polled.map(Some));

    // Boxed: the poll side holds `!Unpin` futures and `select!` needs `Unpin` to call `next()`.
    let mut stream_n = expand_heads(config.start_height, head, heads)
        .skip_while(move |number| {
            futures::future::ready(*number < config.start_height + config.finalization_lag)
        })
        .boxed();

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
                    let lag = config.finalization_lag;
                    let encoding = config.encoding;

                    // Actual block fetching. No more than `max_concurrency` blocks may be
                    // fetched at once.
                    blocks.spawn(async move {
                        eth.get_block(
                            n - lag,
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

/// The height sequence to fetch: `start..=head` up front, then every height newly revealed by
/// a head observation. Observations that do not advance the frontier (repeats, lower heads
/// from a lagging peer) contribute nothing, so subscription and poll can freely overlap.
fn expand_heads<H>(
    start: attestor_primitives::Height,
    head: attestor_primitives::Height,
    heads: H,
) -> impl futures::Stream<Item = attestor_primitives::Height>
where
    H: futures::Stream<Item = attestor_primitives::Height>,
{
    use futures::StreamExt as _;
    futures::stream::iter(start..=head).chain(
        heads
            .scan(head + 1, |next, observed| {
                if observed >= *next {
                    let missing = *next..=observed;
                    *next = observed + 1;
                    futures::future::ready(Some(futures::stream::iter(missing)))
                } else {
                    #[allow(clippy::reversed_empty_ranges)]
                    futures::future::ready(Some(futures::stream::iter(1..=0)))
                }
            })
            .flatten(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;

    #[tokio::test]
    async fn expand_heads_fills_gaps_and_ignores_non_advancing_observations() {
        let heads = futures::stream::iter(vec![5u64, 5, 7, 6, 10]);
        let got: Vec<u64> = expand_heads(3, 5, heads).collect().await;
        assert_eq!(got, vec![3, 4, 5, 6, 7, 8, 9, 10]);
    }

    #[tokio::test]
    async fn expand_heads_starts_immediately_from_the_seeded_head() {
        // No subsequent observation at all: catch-up to the seeded head still happens.
        let got: Vec<u64> = expand_heads(10, 12, futures::stream::pending::<u64>())
            .take(3)
            .collect()
            .await;
        assert_eq!(got, vec![10, 11, 12]);
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
