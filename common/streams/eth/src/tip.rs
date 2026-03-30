#[derive(builder::Builder, Clone)]
pub struct Config {
    pub client: eth::Client,
    pub finalization_lag: attestor_primitives::Height,
    pub start_height: attestor_primitives::Height,
}

/// Follows the latest Eth chain tip, backed by [`eth::Client`] under the hood.
///
/// Implements capped exponential retry without unbounded attempts in order to handle RPC
/// disconnections. This stream can be considered infinite and will never return [`None`].
pub struct StreamTip {
    stream: sync_wrapper::SyncStream<stream_util::BoxedStream<attestor_primitives::Height>>,
    config: Config,
}

impl StreamTip {
    pub async fn new(mut config: Config) -> Self {
        use futures::StreamExt as _;

        let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
            .max_delay(std::time::Duration::from_millis(5_000))
            .map(tokio_retry::strategy::jitter);

        let reconnect = || {
            let client = config.client.clone();
            let start_height = config.start_height;

            async move {
                let stream = client
                    .subscribe()
                    .await?
                    .skip_while(move |header| futures::future::ready(header.number < start_height))
                    .boxed();

                Ok::<_, eth::Error>(stream)
            }
        };

        let retry = tokio_retry::Retry::spawn(strategy, reconnect);
        let mut stream_headers = retry.await.expect("Unbounded retry cannot error");

        let backup = config.clone();

        let stream = async_stream::stream! {
            let mut tip = None;

            loop {
                match stream_headers.next().await {
                    Some(header) => {
                        if let Some(tip_new) = header.number.checked_sub(config.finalization_lag) {
                            if tip.is_none_or(|tip| tip_new > tip) {
                                tip = Some(tip_new);
                                yield tip_new
                            }
                        }
                    },
                    None => {
                        tracing::error!("Eth connection lost");

                        let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
                            .max_delay(std::time::Duration::from_millis(5_000))
                            .map(tokio_retry::strategy::jitter);

                        let reconnect = || {
                            tracing::warn!("Reconnecting to Eth...");

                            let mut client = config.client.clone();
                            let start_height = config.start_height;
                            async move {
                                client.reconnect().await?;

                                let stream = client
                                    .subscribe()
                                    .await?
                                    .skip_while(move |header| {
                                        futures::future::ready(header.number < start_height)
                                    })
                                    .boxed();

                                Ok::<_, eth::Error>((client, stream))
                            }
                        };

                        let retry = tokio_retry::Retry::spawn(strategy, reconnect);
                        let (client, stream) = retry.await.expect("Unbounded retry cannot error");

                        config.client = client;
                        stream_headers = stream;
                    },
                }
            }
        }
        .boxed();

        Self {
            stream: sync_wrapper::SyncStream::new(stream),
            config: backup,
        }
    }
}

impl futures::Stream for StreamTip {
    type Item = attestor_primitives::Height;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;
        self.stream.poll_next_unpin(cx)
    }
}

impl stream_util::ChainData<attestor_primitives::Height> for StreamTip {
    async fn reset(&self, n: u64) -> Self {
        let mut config = self.config.clone();
        config.start_height = n;

        Self::new(config).await
    }
}
