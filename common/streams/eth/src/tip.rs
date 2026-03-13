use crate::Error;

#[derive(builder::Builder)]
pub struct Config {
    pub client: eth::Client,
    pub finalization_lag: attestor_primitives::Height,
}

pub struct StreamTip {
    stream: stream_util::BoxedStream<attestor_primitives::Height>,
}

impl StreamTip {
    pub async fn new(config: Config) -> Result<Self, Error> {
        use futures::StreamExt as _;

        let mut stream_headers = config.client.subscribe().await.map_err(Error::Client)?;

        let stream = async_stream::stream! {
            let mut tip = None;

            while let Some(header) = stream_headers.next().await {
                if tip.is_none_or(|tip| header.number > tip) {
                    if let Some(tip_new) = header.number.checked_sub(config.finalization_lag) {
                        tip = Some(tip_new);
                        yield tip_new
                    }
                }
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
