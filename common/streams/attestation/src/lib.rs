mod error;

pub use error::Error;

#[derive(builder::Builder)]
pub struct Config {
    eth: stream_eth::Config,
    interval_attestation: std::num::NonZero<attestor_primitives::Height>,
    digest_prev: Option<attestor_primitives::Digest>,
    max_catchup: std::num::NonZeroUsize,
}

pub struct Permit(attestor_primitives::Height);

pub struct StreamAttestation {
    roots: stream_eth::StreamRoots,
    cache: Vec<stream_eth::RootInfo>,
    max_catchup: std::num::NonZeroUsize,
    interval_attestation: std::num::NonZero<attestor_primitives::Height>,

    stop: attestor_primitives::Height,
    cursor: Option<attestor_primitives::Height>,

    waker: Option<std::task::Waker>,
}

impl StreamAttestation {
    pub async fn new(config: Config) -> Result<Self, Error> {
        use futures::TryStreamExt as _;

        let mut roots = stream_eth::StreamRoots::new(config.eth)
            .await
            .map_err(Error::Eth)?;
        let mut next = roots
            .try_next()
            .await
            .map_err(Error::Eth)?
            .ok_or(Error::EndOfStream)?;

        let mut cache = Vec::with_capacity(config.max_catchup.get());
        cache.push(next);

        Ok(Self {
            roots,
            cache,
            max_catchup: config.max_catchup,
            interval_attestation: config.interval_attestation,

            stop: 0,
            cursor: None,

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
        use futures::TryStreamExt as _;

        while self.cache.len() < self.max_catchup.get() {
            match self.roots.try_poll_next_unpin(cx) {
                std::task::Poll::Ready(Some(Ok(info))) => {
                    self.cache.push(info);
                }
                std::task::Poll::Ready(Some(Err(err))) => {
                    return std::task::Poll::Ready(Some(Err(Error::Eth(err))))
                }
                std::task::Poll::Ready(None) => {
                    return std::task::Poll::Ready(None);
                }
                std::task::Poll::Pending => break,
            }
        }

        if self.cache.is_empty() {
            self.waker = Some(cx.waker().clone());
            return std::task::Poll::Pending;
        }

        let first = self.cache.first().unwrap();
        let last = self.cache.last().unwrap();

        let height = last.height - (last.height % self.interval_attestation.get());

        let Some(next) = self
            .cursor
            .map(|cursor| cursor.min(height))
            .unwrap_or(height)
            .checked_sub(first.height)
            .and_then(|next| (next > self.stop - first.height).then_some(next))
        else {
            self.waker = Some(cx.waker().clone());
            return std::task::Poll::Pending;
        };

        self.cursor = next.checked_sub(self.interval_attestation.get());
        if self.cursor.is_none() {
            self.stop = next;
        }

        std::task::Poll::Ready(Some(Ok(Permit(next))))
    }
}
