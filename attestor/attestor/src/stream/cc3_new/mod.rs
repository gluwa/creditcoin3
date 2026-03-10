use crate::prelude::*;

mod error;
pub use error::Error;

#[derive(Debug, attestor_macro::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
}

pub struct StreamCC3 {
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamEvents, Error>> + Send>>,
}

impl StreamCC3 {
    pub async fn new(config: Config) -> Result<Self, Error> {
        use futures::StreamExt as _;
        use futures::TryFutureExt as _;
        use futures::TryStreamExt as _;

        let mut blocks = config
            .cc3
            .api()
            .blocks()
            .subscribe_finalized()
            .await
            .map_err(Error::Subxt)?
            .map_err(Error::Subxt)
            .and_then(move |block| StreamEvents::new(block, config.chain_key))
            .boxed();
        let stream = async_stream::stream! {
            let mut latest = None;

            loop {
                match blocks.try_next().await {
                    Ok(Some(event)) => {
                        latest = Some(event.block_number());
                        yield event
                    },
                    Ok(None) | Err(_) => {
                        let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
                            .max_delay(std::time::Duration::from_millis(5_000))
                            .map(tokio_retry::strategy::jitter);

                        blocks = tokio_retry::Retry::spawn(strategy, || {
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
                        })
                        .await
                        .unwrap()
                        .map_err(Error::Subxt)
                        .and_then(move |block| StreamEvents::new(block, config.chain_key))
                        .boxed();
                    },
                }
            }
        }
        .boxed();

        todo!()

        // let stream = config
        //     .cc3
        //     .api()
        //     .blocks()
        //     .subscribe_finalized()
        //     .await
        //     .map_err(Error::Subxt)?
        //     .map_err(Error::Subxt)
        //     .and_then(move |block| StreamEvents::new(block, config.chain_key))
        //     .boxed();
        //
        // Ok(Self { stream })
    }
}

impl futures::Stream for StreamCC3 {
    type Item = Result<StreamEvents, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.stream.as_mut().poll_next(cx)
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
    pub async fn new(
        block: common::types::SubxtBlock,
        chain_key: attestor_primitives::ChainKey,
    ) -> Result<Self, Error> {
        use futures::TryStreamExt as _;

        let block_number = block.number() as common::types::Height;
        let events = block.events().await.map_err(Error::Subxt)?;
        let stream = Box::pin(
            futures::stream::iter(cc_client::Client::extract_events(chain_key, &events))
                .map_err(|err| Error::Subxt(err.into())),
        );

        Ok(Self {
            block_number,
            stream,
        })
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
