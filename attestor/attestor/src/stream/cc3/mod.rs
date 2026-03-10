use crate::prelude::*;

mod error;
pub use error::Error;

#[derive(Debug, attestor_macro::Builder)]
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
        use futures::TryFutureExt as _;
        use futures::TryStreamExt as _;

        let mut events = config
            .cc3
            .api()
            .blocks()
            .subscribe_finalized()
            .await
            .map_err(Error::Subxt)?
            .map_err(Error::Subxt)
            .and_then(move |block| {
                let block_number = block.number() as common::types::Height;
                let chain_key = config.chain_key;

                async move {
                    match block.events().await {
                        Ok(events) => Ok(StreamEvents::new(block_number, events, chain_key)),
                        Err(err) => Err(Error::Subxt(err)),
                    }
                }
            })
            .boxed();
        let next = events.try_next().await?.ok_or(Error::EndOfStream)?;

        let stream = async_stream::stream! {
            let mut events = events;
            let mut latest = next.block_number();

            yield next;

            loop {
                match events.try_next().await {
                    Ok(Some(event)) => {
                        latest = event.block_number();
                        yield event
                    },
                    Ok(None) | Err(_) => {
                        events = 'retry: loop {
                            let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
                                .max_delay(std::time::Duration::from_millis(5_000))
                                .map(tokio_retry::strategy::jitter);
                            let reconnect = || {
                                let mut cc3 = config.cc3.clone();
                                async move {
                                    cc3.reconnect()
                                        .map_err(Error::Client)
                                        .and_then(|cc3| {
                                            cc3.api()
                                                .blocks()
                                                .subscribe_finalized()
                                                .map_err(Error::Subxt)
                                        })
                                        .await
                                }
                            };

                            let retry = tokio_retry::Retry::spawn(strategy, reconnect);
                            let Ok(mut finalized) = retry.await else {
                                continue 'retry;
                            };

                            let Ok(Some(next)) = finalized.try_next().await else {
                                continue 'retry;
                            };

                            break futures::stream::iter(latest + 1..next.number() as u64)
                                .then(|n| {
                                    let legacy = config.cc3.legacy().clone();
                                    let api = config.cc3.api().clone();
                                    let number = subxt::backend::legacy::rpc_methods::NumberOrHex::Number(n);

                                    async move {
                                        match legacy.chain_get_block_hash(Some(number)).await {
                                            Ok(Some(hash)) => api.blocks().at(hash).await.map_err(Error::Subxt),
                                            Ok(None) => Err(Error::BlockHash(n)),
                                            Err(err) => Err(Error::Subxt(err)),
                                        }
                                    }
                                })
                                .chain(futures::stream::once(futures::future::ok(next)))
                                .chain(finalized.map_err(Error::Subxt))
                                .and_then(|block| {
                                    let block_number = block.number() as common::types::Height;
                                    let chain_key = config.chain_key;

                                    async move {
                                        match block.events().await {
                                            Ok(events) => Ok(StreamEvents::new(block_number, events, chain_key)),
                                            Err(err) => Err(Error::Subxt(err)),
                                        }
                                    }
                                })
                                .boxed();
                        }
                    },
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
    block_number: common::types::Height,
}

impl StreamEvents {
    pub fn new(
        block_number: common::types::Height,
        events: subxt::events::Events<subxt::SubstrateConfig>,
        chain_key: attestor_primitives::ChainKey,
    ) -> Self {
        use futures::TryStreamExt as _;

        let stream = Box::pin(
            futures::stream::iter(cc_client::Client::extract_events(chain_key, &events))
                .map_err(|err| Error::Subxt(err.into())),
        );

        Self {
            block_number,
            stream,
        }
    }

    pub fn block_number(&self) -> common::types::Height {
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
