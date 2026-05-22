mod error;
use error::Error;

#[derive(Debug, builder::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_keys: Vec<attestor_primitives::ChainKey>,
}

pub struct StreamCC3 {
    stream: std::pin::Pin<
        Box<dyn futures::Stream<Item = Result<StreamEvents, cc_client::Error>> + Send>,
    >,
}

impl StreamCC3 {
    pub async fn new(mut config: Config) -> Result<Self, cc_client::Error> {
        use arc_swap::access::Access as _;
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        let blocks = config.cc3.api().load().blocks();
        let mut latest = blocks.at_latest().await?.number();
        let mut finalized = blocks.subscribe_finalized().await?;

        let mut backfill = Vec::with_capacity(16);
        let mut err = Ok(());

        let stream = async_stream::stream! {
            'retry: loop {
                match std::mem::replace(&mut err, Ok(())) {
                    Err(Error::Client(cc_client::Error::ConnectionError(reconnect))) => {
                        config.cc3.reconnect(reconnect).await;

                        let blocks = config.cc3.api().load().blocks();
                        finalized = match blocks.subscribe_finalized().await {
                            Ok(finalized_new) => finalized_new,
                            Err(err_new) => {
                                tracing::warn!(?err_new, "Failed to re-subscribe to CC3");
                                err = Err(Error::Client(err_new.into()));
                                continue 'retry;
                            }
                        };

                        config.cc3.reset_connection_delay();
                    }

                    Err(Error::Client(err)) => yield Err(err),

                    Err(Error::EndOfStream) => {
                        let blocks = config.cc3.api().load().blocks();
                        finalized = match blocks.subscribe_finalized().await {
                            Ok(finalized_new) => finalized_new,
                            Err(err_new) => {
                                tracing::warn!(?err_new, "Failed to re-subscribe to CC3");
                                err = Err(Error::Client(err_new.into()));
                                continue 'retry;
                            }
                        };
                    }

                    Ok(_) => {}
                };
                match finalized.try_next().await {
                    Ok(Some(block)) => {
                        let events = match block.events().await {
                            Ok(events) => events,
                            Err(err_new) => {
                                err = Err(Error::Client(err_new.into()));
                                continue 'retry;
                           }
                        };

                        let mut n = block.number();
                        let mut parent_hash = block.header().parent_hash;

                        if n > latest {
                            backfill.push((n, events));

                            // Don't include `latest` again as block parent
                            while n > latest + 1 {
                                let blocks = config.cc3.api().load().blocks();
                                let parent = blocks.at(parent_hash).await;
                                let parent = match parent {
                                    Ok(parent) => parent,
                                    Err(err_new) => {
                                        err = Err(Error::Client(err_new.into()));
                                        backfill.clear();
                                        continue 'retry;
                                    }
                                };
                                let events = match parent.events().await {
                                    Ok(events) => events,
                                    Err(err_new) => {
                                        err = Err(Error::Client(err_new.into()));
                                        backfill.clear();
                                        continue 'retry;
                                    }
                                };

                                n = parent.number();
                                parent_hash = parent.header().parent_hash;

                                backfill.push((n, events));
                            }

                            latest = block.number();
                            for (block_number, events) in backfill.drain(..).rev() {
                                yield Ok(StreamEvents::new(
                                    block_number as attestor_primitives::Height,
                                    events,
                                    config.chain_keys.clone()
                                ));
                            }
                        }
                    },
                    Ok(None) => err = Err(Error::EndOfStream),
                    Err(err_new) => err = Err(Error::Client(err_new.into()))
                }
            }
        }
        .boxed();

        Ok(Self { stream })
    }
}

impl futures::Stream for StreamCC3 {
    type Item = Result<StreamEvents, cc_client::Error>;

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
            dyn futures::Stream<Item = Result<cc_client::events::CcEvent, cc_client::Error>>
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
        chain_keys: Vec<attestor_primitives::ChainKey>,
    ) -> Self {
        use futures::TryStreamExt as _;

        // Collect so the boxed stream is `'static` (extract_events borrows `events`).
        let extracted: Vec<_> = cc_client::Client::extract_events(&chain_keys, &events).collect();

        let stream = Box::pin(
            futures::stream::iter(extracted)
                .map_err(Into::<subxt::Error>::into)
                .map_err(Into::<cc_client::Error>::into),
        );

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
    type Item = Result<cc_client::events::CcEvent, cc_client::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.stream.as_mut().poll_next(cx)
    }
}
