mod error;

use user::prelude::*;

pub use error::Error;

#[derive(builder::Builder)]
pub struct Config {
    eth: eth::Client,
    start_height: attestor_primitives::Height,
    finalization_lag: attestor_primitives::Height,
    max_concurrency: std::num::NonZeroUsize,
    max_parallelism: std::num::NonZeroUsize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RootInfo {
    pub height: attestor_primitives::Height,
    pub digest: attestor_primitives::Digest,
}

pub struct StreamRoots;
impl StreamRoots {
    pub async fn new(
        config: Config,
    ) -> Result<impl futures::Stream<Item = Result<attestor_primitives::Digest, Error>>, Error>
    {
        use futures::StreamExt as _;

        let start_height = config.start_height;
        let max_parallelism = config.max_parallelism.get();
        let stream_blocks = StreamRpc::new(config).await?;

        let mut next = start_height;
        let mut roots = tokio::task::JoinSet::<RootInfo>::new();
        let mut heap = std::collections::BinaryHeap::with_capacity(max_parallelism);

        Ok(async_stream::stream! {
            let mut stream_blocks = std::pin::pin!(stream_blocks);

            loop {
                tokio::select! {
                    Some(block) = stream_blocks.next() => {
                        match block {
                            Ok(block) => {
                                while roots.len() >= max_parallelism {
                                    if let Some(root) = roots.join_next().await {
                                        match root.map_err(Error::Task) {
                                            Ok(info) => {
                                                heap.push(std::cmp::Reverse(info));
                                                if heap
                                                    .peek()
                                                    .is_some_and(|info| info.0.height == next)
                                                {
                                                    next += 1;
                                                    yield Ok(heap
                                                        .pop()
                                                        .expect("Checked above")
                                                        .0
                                                        .digest);
                                                }
                                            },
                                            Err(err) => yield Err(err),
                                        }
                                    }
                                }

                                roots.spawn_blocking(move || {
                                    RootInfo {
                                        height: block.number(),
                                        digest: eth::simple_merkle_tree(&block).root()
                                    }
                                });
                            },
                            Err(err) => yield Err(err),
                        }
                    }
                    Some(root) = roots.join_next(), if !roots.is_empty() => {
                        match root.map_err(Error::Task) {
                            Ok(info) => {
                                heap.push(std::cmp::Reverse(info));
                                while heap.peek().is_some_and(|info| info.0.height == next) {
                                    next += 1;
                                    yield Ok(heap.pop().expect("Checked above").0.digest);
                                }
                            },
                            Err(err) => yield Err(err),
                        }
                    }
                    else => {
                        break;
                    }
                }
            }
        })
    }
}

struct StreamRpc;
impl StreamRpc {
    pub async fn new(
        config: Config,
    ) -> Result<impl futures::Stream<Item = Result<eth::OrderedBlock, Error>>, Error> {
        use futures::StreamExt as _;

        let mut stream_headers = config.eth.subscribe().await.map_err(Error::Client)?;
        let next = stream_headers.next().await.ok_or(Error::StreamEnd)?.number;

        let mut stream_n = futures::stream::iter(config.start_height..=next)
            .chain(stream_headers.map(|header| header.number))
            .skip_while(move |number| {
                futures::future::ready(
                    *number < config.finalization_lag
                        || *number < config.start_height + config.finalization_lag,
                )
            });

        let mut blocks = tokio::task::JoinSet::new();

        Ok(async_stream::stream! {
            loop {
                tokio::select! {
                    Some(n) = stream_n.next() => {
                        while blocks.len() >= config.max_concurrency.get() {
                            if let Some(block) = blocks.join_next().await {
                                match block.map_err(Error::Task)
                                {
                                    Ok(Ok(block)) => yield Ok(block),
                                    Ok(Err(Interrupt::Cont(err))) => yield Err(err),
                                    Ok(Err(Interrupt::Stop)) => break,
                                    Err(err) => yield Err(err),
                                }
                            }
                        }

                        let eth = config.eth.clone();
                        let lag = config.finalization_lag;

                        blocks.spawn(async move {
                            eth.get_block(
                                n - lag,
                                usc_abi_encoding::common::EncodingVersion::V1
                            )
                            .await
                            .map_interrupt(Error::Client)
                        });
                    }
                    Some(block) = blocks.join_next(), if !blocks.is_empty() => {
                        match block.map_err(Error::Task)
                        {
                            Ok(Ok(block)) => yield Ok(block),
                            Ok(Err(Interrupt::Cont(err))) => yield Err(err),
                            Ok(Err(Interrupt::Stop)) => break,
                            Err(err) => yield Err(err),
                        }
                    }
                    else => {
                        break;
                    }
                }
            }
        })
    }
}
