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
    stream: stream_util::BoxedStream<stream_util::RootInfo>,
}

impl StreamRoots {
    pub async fn new(mut config: Config) -> Result<Self, Error> {
        use futures::StreamExt as _;

        let start_height = config.start_height;
        let max_parallelism = config.max_parallelism.get();
        let mut stream_blocks = stream_rpc(config.clone()).await?;

        let mut next = start_height;
        let mut roots = tokio::task::JoinSet::<stream_util::RootInfo>::new();
        let mut heap = std::collections::BinaryHeap::with_capacity(max_parallelism);

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

        Ok(Self { stream })
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
                let stream = stream_rpc(config.clone()).await?;

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

async fn stream_rpc(
    config: Config,
) -> Result<stream_util::BoxedStream<Result<eth::OrderedBlock, Error>>, Error> {
    use futures::StreamExt as _;

    let mut stream_headers = config.client.subscribe().await.map_err(Error::Client)?;
    let next = stream_headers.next().await.ok_or(Error::StreamEnd)?.number;

    let mut stream_n = futures::stream::iter(config.start_height..=next)
        .chain(stream_headers.map(|header| header.number))
        .skip_while(move |number| {
            futures::future::ready(*number < config.start_height + config.finalization_lag)
        });

    let mut blocks = tokio::task::JoinSet::new();

    Ok(async_stream::stream! {
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
                                Ok(Err(Interrupt::Stop)) => break,
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

                    // Actual block fetching. No more than `max_concurrency` blocks may be
                    // fetched at once.
                    blocks.spawn(async move {
                        eth.get_block(
                            n - lag,
                            usc_abi_encoding::common::EncodingVersion::V1
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
                        Ok(Err(Interrupt::Stop)) => break,
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
    .boxed())
}
