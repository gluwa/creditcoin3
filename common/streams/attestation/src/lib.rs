mod error;

pub use error::Error;

#[derive(builder::Builder)]
pub struct Config {
    eth: stream_eth::roots::Config,
    interval_attestation: std::num::NonZero<attestor_primitives::Height>,
    digest_prev: Option<attestor_primitives::Digest>,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
}

#[derive(Debug)]
pub struct Permit(attestor_primitives::Height);

pub struct StreamAttestation {
    stream_roots: stream_eth::StreamRoots,
    stream_tip: stream_eth::StreamTip,

    cache: Vec<stream_eth::RootInfo>,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
    interval_attestation: std::num::NonZero<attestor_primitives::Height>,

    missing: std::ops::RangeInclusive<attestor_primitives::Height>,
    tip: attestor_primitives::Height,
    cursor: attestor_primitives::Height,

    waker: Option<std::task::Waker>,
}

impl StreamAttestation {
    pub async fn new(mut config: Config) -> Result<Self, Error> {
        use futures::StreamExt as _;

        config.eth.start_height =
            config.eth.start_height - (config.eth.start_height % config.interval_attestation.get());

        let config_tip = stream_eth::tip::ConfigBuilder::new()
            .with_client(config.eth.client.clone())
            .with_finalization_lag(config.eth.finalization_lag)
            .build();
        let mut stream_tip = stream_eth::tip::StreamTip::new(config_tip)
            .await
            .map_err(Error::Eth)?;

        let tip = stream_tip.next().await.ok_or(Error::EndOfStream)?;
        let tip = tip - (tip % config.interval_attestation.get());

        let start = config.eth.start_height;
        let stop = start
            .saturating_add(config.max_catchup.get())
            .min(tip - (tip % config.interval_attestation.get()));
        let missing = start..=stop;

        let stream_roots = stream_eth::StreamRoots::new(config.eth)
            .await
            .map_err(Error::Eth)?;

        let cache = Vec::with_capacity(config.max_catchup.get() as usize);

        Ok(Self {
            stream_roots,
            stream_tip,

            cache,
            max_catchup: config.max_catchup,
            interval_attestation: config.interval_attestation,

            missing,
            tip,
            cursor: stop,

            waker: None,
        })
    }
}

impl futures::Stream for StreamAttestation {
    type Item = Result<Permit, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;

        loop {
            tracing::info!(
                cursor = self.cursor,
                start = self.missing.start(),
                "Polling for permit"
            );

            if &self.cursor > self.missing.start() {
                let permit = Permit(self.cursor);
                self.cursor = self.cursor.saturating_sub(self.interval_attestation.get());
                return std::task::Poll::Ready(Some(Ok(permit)));
            }

            if self.cache.len() >= self.max_catchup.get() as usize {
                tracing::error!("Max size reached");
                self.waker = Some(cx.waker().clone());
                return std::task::Poll::Pending;
            }

            let start = self
                .missing
                .end()
                .to_owned()
                .saturating_add(self.interval_attestation.get());

            tracing::info!(start, missing = ?self.missing, "Updating tip");

            while self.tip < start {
                tracing::info!(tip = self.tip, "New tip");
                self.tip = match std::task::ready!(self.stream_tip.poll_next_unpin(cx)) {
                    Some(tip) => tip - (tip % self.interval_attestation.get()),
                    None => return std::task::Poll::Ready(None),
                };
            }

            while let Some(info) = std::task::ready!(self.stream_roots.poll_next_unpin(cx)) {
                match info {
                    Ok(info) => {
                        tracing::info!(?info, "Computed root info");

                        self.cache.push(info.clone());

                        if info.height == self.tip
                            || self.cache.len() >= self.max_catchup.get() as usize
                        {
                            break;
                        }
                    }
                    Err(err) => return std::task::Poll::Ready(Some(Err(Error::Eth(err)))),
                }
            }

            let stop = self
                .missing
                .end()
                .saturating_add(self.max_catchup.get())
                .min(self.tip);

            self.missing = *self.missing.end()..=stop;
            self.cursor = stop;

            tracing::info!(cursor = self.cursor, missing = ?self.missing, "Reached tip");
        }
    }
}
