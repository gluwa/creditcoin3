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
        use futures::TryStreamExt as _;

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

        let mut stream_roots = stream_eth::StreamRoots::new(config.eth)
            .await
            .map_err(Error::Eth)?;

        let mut cache = Vec::with_capacity(config.max_catchup.get() as usize);

        while let Some(info) = stream_roots.try_next().await.map_err(Error::Eth)? {
            let height = info.height;
            cache.push(info);

            if height >= stop {
                break;
            }
        }

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
        use futures::TryStreamExt as _;

        loop {
            if &self.cursor > self.missing.start() {
                let permit = Permit(self.cursor);
                self.cursor = self.cursor.saturating_sub(self.interval_attestation.get());
                return std::task::Poll::Ready(Some(Ok(permit)));
            }

            if self.cache.len() >= self.max_catchup.get() as usize {
                self.waker = Some(cx.waker().clone());
                return std::task::Poll::Pending;
            }

            let next = self
                .missing
                .end()
                .to_owned()
                .saturating_add(self.interval_attestation.get());

            while (self.tip < next
                || self
                    .cache
                    .last()
                    .map(|info| info.height)
                    .unwrap_or_default()
                    < self.tip)
                && self.cache.len() < self.max_catchup.get() as usize
            {
                let mut progress = false;

                match self.stream_roots.try_poll_next_unpin(cx) {
                    std::task::Poll::Ready(Some(Ok(info))) => {
                        self.cache.push(info);
                        progress = true;
                    }
                    std::task::Poll::Ready(Some(Err(err))) => {
                        return std::task::Poll::Ready(Some(Err(Error::Eth(err))))
                    }
                    std::task::Poll::Ready(None) => {}
                    std::task::Poll::Pending => {}
                }

                match self.stream_tip.poll_next_unpin(cx) {
                    std::task::Poll::Ready(Some(tip)) => {
                        self.tip = tip;
                        progress = true;
                    }
                    std::task::Poll::Ready(None) => {}
                    std::task::Poll::Pending => {}
                }

                if !progress {
                    return std::task::Poll::Pending;
                }
            }

            let stop = self
                .missing
                .end()
                .saturating_add(self.max_catchup.get())
                .min(self.tip);

            self.missing = *self.missing.end()..=stop;
            self.cursor = stop;
        }
    }
}
