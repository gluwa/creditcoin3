mod error;
pub use error::Error;

#[derive(Debug, builder::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
}

pub struct StreamCC3 {
    stream: std::pin::Pin<Box<dyn futures::Stream<Item = StreamEvents> + Send>>,
}

impl StreamCC3 {
    pub async fn new(mut config: Config) -> Result<Self, Error> {
        use arc_swap::access::Access as _;
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        let blocks = config.cc3.api().load().blocks();
        let mut latest = blocks.at_latest().await.map_err(Error::Subxt)?.number();
        let mut finalized = blocks.subscribe_finalized().await.map_err(Error::Subxt)?;

        let mut backfill = Vec::with_capacity(16);
        let mut err = Ok(());

        let stream = async_stream::stream! {
            'retry: loop {
                match err {
                    Err(Error::Subxt(ref err_new)) => {
                        config.cc3.reconnect(&err_new).await;

                        let blocks = config.cc3.api().load().blocks();
                        finalized = match blocks.subscribe_finalized().await {
                            Ok(finalized_new) => finalized_new,
                            Err(err_new) => {
                                tracing::warn!(?err_new, "Failed to re-subsribe to CC3");
                                continue 'retry;
                            }
                        };

                        config.cc3.reset_connection_delay();
                    }
                    Err(err) => panic!("Unrecoverable error: {err}"),
                    Ok(_) => {}
                };
                match finalized.try_next().await {
                    Ok(Some(block)) => {
                        let events = match block.events().await {
                            Ok(events) => events,
                            Err(err_new) => {
                                err = Err(Error::Subxt(err_new));
                                continue 'retry;
                           }
                        };

                        let mut n = block.number();
                        let mut parent_hash = block.header().parent_hash;

                        backfill.push((n, events));

                        while n > latest {
                            let blocks = config.cc3.api().load().blocks();
                            let parent = blocks.at(parent_hash).await;
                            let parent = match parent {
                                Ok(parent) => parent,
                                Err(err_new) => {
                                    err = Err(Error::Subxt(err_new));
                                    continue 'retry;
                                }
                            };
                            let events = match parent.events().await {
                                Ok(events) => events,
                                Err(err_new) => {
                                    err = Err(Error::Subxt(err_new));
                                    continue 'retry;
                                }
                            };

                            n = parent.number();
                            parent_hash = parent.header().parent_hash;

                            backfill.push((n, events));
                        }

                        latest = block.number();
                        for (block_number, events) in backfill.drain(..).rev() {
                            yield StreamEvents::new(
                                block_number as attestor_primitives::Height,
                                events,
                                config.chain_key
                            );
                        }
                    },
                    Ok(None) => err = Err(Error::EndOfStream),
                    Err(err_new) => err = Err(Error::Subxt(err_new))
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
        Box<dyn futures::Stream<Item = Result<cc_client::events::CcEvent, Error>> + Send + Sync>,
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
    type Item = Result<cc_client::events::CcEvent, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.stream.as_mut().poll_next(cx)
    }
}
