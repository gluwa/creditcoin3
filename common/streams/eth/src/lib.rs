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

struct StreamRpc {
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<eth::OrderedBlock, Error>> + Send>>,
}

impl StreamRpc {
    pub async fn new(config: Config) -> Result<Self, Error> {
        use futures::StreamExt as _;

        let mut headers = config.eth.subscribe().await.map_err(Error::Client)?;
        let next = headers.next().await.ok_or(Error::StreamEnd)?.number;

        let mut numbers = futures::stream::iter(config.start_height..=next)
            .chain(headers.map(|header| header.number))
            .skip_while(move |number| {
                futures::future::ready(
                    *number < config.finalization_lag
                        || *number < config.start_height + config.finalization_lag,
                )
            });

        let mut blocks = tokio::task::JoinSet::new();

        Ok(StreamRpc {
            stream: async_stream::stream! {
                loop {
                    tokio::select! {
                        Some(n) = numbers.next() => {
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
                        Some(block) = blocks.join_next() => {
                            match block.map_err(Error::Task)
                            {
                                Ok(Ok(block)) => yield Ok(block),
                                Ok(Err(Interrupt::Cont(err))) => yield Err(err),
                                Ok(Err(Interrupt::Stop)) => break,
                                Err(err) => yield Err(err),
                            }
                        }
                    }
                }
            }
            .boxed(),
        })
    }
}
