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

pub struct StreamAttestation {
    cc3: cc_client::Client,
    chain_key: attestor_primitives::ChainKey,
    bls_key: bls_signatures::PrivateKey,

    stream_roots: stream_util::BoxedStream<stream_util::RootInfo>,
    stream_tip: stream_util::BoxedStream<attestor_primitives::Height>,

    cache: Vec<stream_util::RootInfo>,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
    interval_attestation: std::num::NonZero<attestor_primitives::Height>,
    digest_prev: attestor_primitives::Digest,

    missing: std::ops::RangeInclusive<attestor_primitives::Height>,
    tip: attestor_primitives::Height,
    cursor: attestor_primitives::Height,

    waker: Option<std::task::Waker>,
}

impl StreamAttestation {
    pub fn new(config: Config) -> Self {
        let cache = Vec::with_capacity(config.max_catchup.get() as usize);

        Self {
            cc3: config.cc3,
            chain_key: config.chain_key,
            bls_key: config.bls_key,

            stream_roots: config.stream_roots,
            stream_tip: config.stream_tip,

            cache,
            max_catchup: config.max_catchup,
            interval_attestation: config.interval_attestation,
            digest_prev: config.digest_prev,

            missing: 0..=0,
            tip: 0,
            cursor: 0,

            waker: None,
        }
    }

    pub fn generate_attestation(&self, Permit(target): Permit) -> Attestation {
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

    pub async fn generate_attestation_genesis(
        &self,
        stream_util::RootInfo { height, root, hash }: stream_util::RootInfo,
    ) -> Result<Attestation, Interrupt<Error>> {
        Ok(self.sign_attestation(
            attestor_primitives::AttestationData::new(self.chain_key, height, hash, root, None),
            Default::default(),
        ))
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
            < self.tip
    }

    fn has_space_left(&self) -> bool {
        self.cache.len() <= self.max_catchup.get() as usize
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
            if self.cache.len() > self.max_catchup.get() as usize {
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
