mod error;

pub use error::Error;

use user::*;

pub type Attestation =
    attestor_primitives::Attestation<attestor_primitives::Digest, attestor_primitives::AttestorId>;

#[derive(builder::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
    bls_key: bls_signatures::PrivateKey,

    stream_roots: stream_util::BoxedStream<stream_util::RootInfo>,
    stream_tip: stream_util::BoxedStream<attestor_primitives::Height>,

    interval_attestation: std::num::NonZero<attestor_primitives::Height>,
    digest_prev: attestor_primitives::Digest,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
}

#[derive(Debug)]
pub struct Permit(attestor_primitives::Height);

impl Permit {
    pub fn height(&self) -> attestor_primitives::Height {
        self.0
    }
}

/// A generic attestation stream. Different source chains can be configured by passing in different
/// data streams for retrieving the chain tip ([`Config::stream_tip`]) and chain roots
/// ([`Config::stream_roots`]). Just make sure that the streams you are using have the same
/// finalization lag, if any.
///
/// ## Generation
///
/// The stream works by backfilling attestations up to [`Config::max_catchup`]: attestations are
/// generated backwards from the tip of the chain, or the point closest to the tip of the chain
/// given the max catchup. For example, if the max catchup is 500 and the tip of the chain is block
/// 600, the stream will start backfilling from block 500. As new attestations are marked as
/// finalized via [`note_attestation_finalization`], block roots stored inside of the stream are
/// progressively cleaned and new roots are fetched to keep making progress in attestation
/// generation.
///
/// This mode of reverse generation via backfilling is used to always promote the latest
/// attestation, while other attestations are used as backup in case consensus cannot be reached at
/// the target height.
///
/// [`note_attestation_finalization`]: Self::note_attestation_finalization
pub struct StreamAttestation {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
    bls_key: bls_signatures::PrivateKey,

    stream_roots: stream_util::BoxedStream<stream_util::RootInfo>,
    stream_tip: stream_util::BoxedStream<attestor_primitives::Height>,
    fetching: bool,

    cache: Vec<stream_util::RootInfo>,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
    interval_attestation: std::num::NonZero<attestor_primitives::Height>,
    digest_prev: attestor_primitives::Digest,

    /// Range of roots which have been generated and for which we can return a permit. We need to
    /// keep track of this to know how many blocks to fetch next
    missing: std::ops::RangeInclusive<attestor_primitives::Height>,

    /// Source chain tip, updated as we need to fetch more blocks
    tip: attestor_primitives::Height,

    /// Latest attestation to have been produced
    cursor: attestor_primitives::Height,

    waker: Option<std::task::Waker>,
}

impl StreamAttestation {
    pub fn new(config: Config) -> Self {
        let div = config.max_catchup.get() / config.interval_attestation.get();
        let max_catchup = config.interval_attestation.saturating_mul(
            std::num::NonZero::new(div)
                .unwrap_or(std::num::NonZero::<attestor_primitives::Height>::MIN),
        );

        let cache = Vec::with_capacity(max_catchup.get() as usize);

        Self {
            cc3: config.cc3,
            chain_key: config.chain_key,
            bls_key: config.bls_key,

            stream_roots: config.stream_roots,
            stream_tip: config.stream_tip,
            fetching: false,

            cache,
            max_catchup,
            interval_attestation: config.interval_attestation,
            digest_prev: config.digest_prev,

            missing: 0..=0,
            tip: 0,
            cursor: 0,

            waker: None,
        }
    }

    pub fn max_catchup(&self) -> std::num::NonZero<attestor_primitives::Height> {
        self.max_catchup
    }

    pub fn note_attestation_finalization(
        &mut self,
        height: attestor_primitives::Height,
        digest: attestor_primitives::Digest,
    ) {
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
        self.digest_prev = digest;

        if let Some(waker) = self.waker.take() {
            waker.wake()
        }
    }

    pub fn generate_attestation_genesis(
        &self,
        stream_util::RootInfo { height, root, hash }: stream_util::RootInfo,
    ) -> Result<Attestation, Interrupt<Error>> {
        Ok(self.sign_attestation(
            attestor_primitives::AttestationData::new(self.chain_key, height, hash, root, None),
            Default::default(),
        ))
    }

    fn generate_attestation(&self, target: attestor_primitives::Height) -> Attestation {
        assert!(!self.cache.is_empty(), "Empty root cache");

        let block_first = self.cache.first().unwrap().height;
        assert!(target >= block_first, "{target} >= {block_first}");

        let block_last = self.cache.last().unwrap().height;
        assert!(target <= block_last, "{target} <= {block_last}");

        let index = target as usize - block_first as usize;
        let stream_util::RootInfo { height, root, hash } = self.cache[index];

        assert_eq!(height, target, "Attestation height mismatch");

        let blocks = self.cache[0..index].iter().fold(
            Vec::<attestor_primitives::block::BlockSerializable>::with_capacity(index),
            |mut acc, stream_util::RootInfo { height, root, .. }| {
                let digest_prev = acc
                    .last()
                    .map(|block| block.digest)
                    .unwrap_or(self.digest_prev);
                let block = attestor_primitives::block::Block::new_from_prev_digest(
                    *height,
                    *root,
                    digest_prev,
                );

                acc.push(block.into());
                acc
            },
        );

        let continuity_proof =
            attestor_primitives::attestation_fragment::AttestationFragmentSerializable { blocks };

        let attestation_data = attestor_primitives::AttestationData::new(
            self.chain_key,
            target,
            hash,
            root,
            continuity_proof.head().map(|head| head.digest),
        );

        self.sign_attestation(attestation_data, continuity_proof)
    }

    fn sign_attestation(
        &self,
        attestation_data: attestor_primitives::AttestationData<attestor_primitives::Digest>,
        continuity_proof: attestor_primitives::attestation_fragment::AttestationFragmentSerializable,
    ) -> attestor_primitives::Attestation<
        attestor_primitives::Digest,
        attestor_primitives::AttestorId,
    > {
        let attestor = self.cc3.get_attestor_id();
        let message = attestation_data.serialize();
        let signature = sp_core::sr25519::Signature::from_raw(self.cc3.sign(&message).0);
        let signature_bls = attestor_primitives::bls::WrapEncode(self.bls_key.sign(message));

        attestor_primitives::Attestation {
            attestation_data,
            attestor,
            signature,
            signature_bls,
            continuity_proof,
        }
    }

    fn missing_roots(&self) -> bool {
        self.cache
            .last()
            .map(|info| info.height)
            .unwrap_or_default()
            < self
                .missing
                .start()
                .saturating_add(self.max_catchup.get())
                .min(self.tip)
    }

    fn has_space_left(&self) -> bool {
        self.cache.len() <= self.max_catchup.get() as usize
    }
}

impl futures::Stream for StreamAttestation {
    type Item = Result<Attestation, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;

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

                let attestation = self.generate_attestation(self.cursor);
                self.cursor = self.cursor.saturating_sub(self.interval_attestation.get());
                return std::task::Poll::Ready(Some(Ok(attestation)));
            }

            // Backpressure, limit the max number of roots which can be processed into a single
            // attestation
            if !self.fetching && self.cache.len() > self.max_catchup.get() as usize {
                self.waker = Some(cx.waker().clone());
                return std::task::Poll::Pending;
            }

            self.fetching = true;

            let next = self
                .missing
                .end()
                .to_owned()
                .saturating_add(self.interval_attestation.get());

            // Chain tip and roots are polled concurrently until a new attestation can be produced
            while self.tip < next || self.missing_roots() {
                let mut progress = false;

                if self.has_space_left() {
                    match self.stream_roots.poll_next_unpin(cx) {
                        std::task::Poll::Ready(Some(info)) => {
                            self.cache.push(info);
                            progress = true;
                        }
                        std::task::Poll::Ready(None) => {
                            return std::task::Poll::Ready(None);
                        }
                        std::task::Poll::Pending => {}
                    }
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

            self.fetching = false;

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
