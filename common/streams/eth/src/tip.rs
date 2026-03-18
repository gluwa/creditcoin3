use crate::Error;

/// Default timeout for receiving a new block header before treating the
/// connection as stale and forcing a reconnect.  Two minutes is generous
/// enough for any chain with ≤30 s block times while still catching silent
/// drops reasonably fast.
const HEADER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

#[derive(builder::Builder)]
pub struct Config {
    pub client: eth::Client,
    pub finalization_lag: attestor_primitives::Height,
}

/// Follows the latest Eth chain tip, backed by [`eth::Client`] under the hood.
///
/// Implements capped exponential retry without unbounded attempts in order to handle RPC
/// disconnections. This stream can be considered infinite and will never return [`None`].
pub struct StreamTip {
    stream: stream_util::BoxedStream<attestor_primitives::Height>,
}

impl StreamTip {
    pub async fn new(mut config: Config) -> Result<Self, Error> {
        use futures::StreamExt as _;

        let mut stream_headers = config.client.subscribe().await.map_err(Error::Client)?;

        let stream = async_stream::stream! {
            let mut tip = None;

            loop {
                match tokio::time::timeout(HEADER_TIMEOUT, stream_headers.next()).await {
                    Ok(Some(header)) => {
                        if let Some(tip_new) = header.number.checked_sub(config.finalization_lag) {
                            if tip.is_none_or(|tip| tip_new > tip) {
                                tip = Some(tip_new);
                                yield tip_new
                            }
                        }
                        continue;
                    },
                    Ok(None) => {
                        tracing::error!("Eth connection lost");
                    }
                    Err(_timeout) => {
                        tracing::error!(
                            timeout_secs = HEADER_TIMEOUT.as_secs(),
                            "Eth connection stale (no headers received within timeout)"
                        );
                    }
                }

                // Only reached on disconnect or timeout — reconnect.
                let strategy = tokio_retry::strategy::ExponentialBackoff::from_millis(100)
                    .max_delay(std::time::Duration::from_millis(5_000))
                    .map(tokio_retry::strategy::jitter);

                let reconnect = || {
                    tracing::warn!("Reconnecting to Eth...");

                    let mut client = config.client.clone();
                    async move {
                        client.reconnect().await?;
                        let stream = client.subscribe().await?;

                        Ok::<_, eth::Error>((client, stream))
                    }
                };

                let retry = tokio_retry::Retry::spawn(strategy, reconnect);
                let (client, stream) = retry.await.expect("Unbounded retry cannot error");

                config.client = client;
                stream_headers = stream;
            }
        }
        .boxed();

        Ok(Self { stream })
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
