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

impl Permit {
    pub fn height(&self) -> attestor_primitives::Height {
        self.0
    }
}

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
        config.eth.start_height =
            config.eth.start_height - (config.eth.start_height % config.interval_attestation.get());

        let config_tip = stream_eth::tip::ConfigBuilder::new()
            .with_client(config.eth.client.clone())
            .with_finalization_lag(config.eth.finalization_lag)
            .build();
        let stream_tip = stream_eth::tip::StreamTip::new(config_tip)
            .await
            .map_err(Error::Eth)?;

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

            missing: 0..=0,
            tip: 0,
            cursor: 0,

            waker: None,
        })
    }

    pub fn note_attestation_finalization(&mut self, height: attestor_primitives::Height) {
        if !self.cache.is_empty() {
            let first = self.cache.first().expect("Checked above").height as usize;
            let last = self.cache.last().expect("Checked above").height as usize;
            let height = height as usize;

            if height >= first && height <= last {
                let index = height - first;
                self.cache.drain(0..=index);
            }
        }

        self.missing = *self.missing.start().max(&height)..=*self.missing.end();

        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }

    fn missing_roots(&self) -> bool {
        self.cache
            .last()
            .map(|info| info.height)
            .unwrap_or_default()
            < self.tip
    }

    fn has_space_left(&self) -> bool {
        self.cache.len() < self.max_catchup.get() as usize + 1
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
            // Yield cached roots
            if self.cursor > *self.missing.start() {
                assert!(
                    self.cache
                        .last()
                        .is_some_and(|info| info.height >= self.cursor),
                    "Missing block root ({}) in cache ([{:?}; {:?}])",
                    self.cursor,
                    self.cache.first(),
                    self.cache.last()
                );

                let permit = Permit(self.cursor);
                self.cursor = self.cursor.saturating_sub(self.interval_attestation.get());
                return std::task::Poll::Ready(Some(Ok(permit)));
            }

            // Backpressure, limit the max number of roots which can be processed into a single
            // attestation
            if self.cache.len() >= self.max_catchup.get() as usize + 1 {
                self.waker = Some(cx.waker().clone());
                return std::task::Poll::Pending;
            }

            let next = self
                .missing
                .end()
                .to_owned()
                .saturating_add(self.interval_attestation.get());

            // Chain tip and roots are polled concurrently until a new attestation can be produced
            while (self.tip < next || self.missing_roots()) && self.has_space_left() {
                let mut progress = false;

                match self.stream_roots.try_poll_next_unpin(cx) {
                    std::task::Poll::Ready(Some(Ok(info))) => {
                        self.cache.push(info);
                        progress = true;
                    }
                    std::task::Poll::Ready(Some(Err(err))) => {
                        return std::task::Poll::Ready(Some(Err(Error::Eth(err))))
                    }
                    std::task::Poll::Ready(None) => {
                        return std::task::Poll::Ready(None);
                    }
                    std::task::Poll::Pending => {}
                }

                match self.stream_tip.poll_next_unpin(cx) {
                    std::task::Poll::Ready(Some(tip)) => {
                        self.tip = tip - (tip % self.interval_attestation.get());
                        progress = true;
                    }
                    std::task::Poll::Ready(None) => {
                        return std::task::Poll::Ready(None);
                    }
                    std::task::Poll::Pending => {}
                }

                if !progress {
                    return std::task::Poll::Pending;
                }
            }

            assert!(
                self.cache.len() <= self.max_catchup.get() as usize + 1,
                "Cache length ({}) exceeds max_catchup ({})",
                self.cache.len(),
                self.max_catchup
            );

            let stop = self
                .missing
                .start()
                .saturating_add(self.max_catchup.get())
                .min(self.tip);

            self.missing = *self.missing.end()..=stop;
            self.cursor = stop;
        }
    }
}
