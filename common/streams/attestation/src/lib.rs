mod error;

#[cfg(all(test, feature = "simulation"))]
mod simulation;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod prelude {
    pub use crate::nonzero;
    pub use crate::poll;
}

pub use error::Error;

pub type Attestation =
    attestor_primitives::Attestation<attestor_primitives::Digest, attestor_primitives::AttestorId>;

#[derive(builder::Builder)]
pub struct Config {
    signer: cc_client::signer::CC3Signer,
    chain_key: attestor_primitives::ChainKey,
    bls_key: bls_signatures::PrivateKey,

    stream_roots: stream_util::BoxedData<stream_util::RootInfo>,
    stream_tip: stream_util::BoxedData<attestor_primitives::Height>,

    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    attestation_prev: stream_util::AttestationInfo,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
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
/// ## Cancellation Safety
///
/// [`StreamAttestation`] is [cancellation-safe]. The stream makes progress in two steps:
///
/// 1. Attestations are backfilled from the latest possible point.
/// 2. Once backfilling has completed, new block roots are fetched and stored in a local cache.
///
/// Crucially, no new attestations may be returned until the fetching process has completed. This
/// separates storage mutations from stream yielding, so that we cannot enter an invalid state
/// midway during attestations generation. If the stream is interrupted during polling, then
/// restarted, it will either return the next attestation or resume its fetching process.
///
/// [`note_attestation_finalization`]: Self::note_attestation_finalization
/// [cancellation-safe]: https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety
pub struct StreamAttestation {
    signer: cc_client::signer::CC3Signer,
    chain_key: attestor_primitives::ChainKey,
    bls_key: bls_signatures::PrivateKey,

    stream_roots: stream_util::BoxedData<stream_util::RootInfo>,
    stream_tip: stream_util::BoxedData<attestor_primitives::Height>,
    fetching: bool,

    cache: Vec<stream_util::RootInfo>,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    attestation_prev: stream_util::AttestationInfo,

    /// Range of roots which have been computed and can be used to generate an attestation. We keep
    /// track of this to know how many blocks to fetch even if the cache is empty.
    computed: std::ops::RangeInclusive<attestor_primitives::Height>,
    /// Source chain tip, keeps track of advancement during root computation.
    tip: attestor_primitives::Height,
    /// The next attestation to produce in the [`computed`] range.
    ///
    /// [`computed`]: Self::computed
    cursor: attestor_primitives::Height,

    waker: Option<std::task::Waker>,
}

impl StreamAttestation {
    pub fn new(config: Config) -> Self {
        // max catchup is aligned to the attestation interval to simplify attestation generation
        // logic, otherwise several edge cases can occur around the alignment of new block roots.
        let div = config.max_catchup.get() / config.attestation_interval.get();
        let max_catchup = config.attestation_interval.saturating_mul(
            std::num::NonZero::new(div)
                .unwrap_or(std::num::NonZero::<attestor_primitives::Height>::MIN),
        );

        let cache = Vec::with_capacity(max_catchup.get() as usize);

        Self {
            signer: config.signer,
            chain_key: config.chain_key,
            bls_key: config.bls_key,

            stream_roots: config.stream_roots,
            stream_tip: config.stream_tip,
            fetching: false,

            cache,
            max_catchup,
            attestation_interval: config.attestation_interval,
            attestation_prev: config.attestation_prev,

            computed: config.attestation_prev.height..=config.attestation_prev.height,
            tip: 0,
            cursor: 0,

            waker: None,
        }
    }

    pub fn max_catchup(&self) -> std::num::NonZero<attestor_primitives::Height> {
        self.max_catchup
    }

    /// Lets the [`StreamAttestation`] know that a new attestation has finalized on-chain.
    ///
    /// This is a no-op if the stream was already notified of a higher attestation finalizing, or
    /// if the finalized attestation is not at a multiple of the configured attestation interval.
    /// Note that in the latter case this indicates that the stream should be re-generated with
    /// the new interval.
    pub fn note_attestation_finalization(&mut self, info: stream_util::AttestationInfo) {
        if self.attestation_prev.height < info.height {
            // Regenerate attestations after the finalized height
            let end = info.height.max(*self.computed.end());
            self.computed = info.height..=end;
            self.cursor = end;

            // Updates the previous digest
            self.attestation_prev = info;

            // The root cache is drained of past roots which are no longer needed to reach
            // consensus.
            if !self.cache.is_empty() {
                let first = self.cache.first().expect("Checked above").height as usize;
                let height = info.height as usize;

                if height >= first {
                    let index = (height - first).min(self.cache.len() - 1);
                    self.cache.drain(0..=index);
                }
            }

            // It is possible that after these updates the attestation stream is now able to
            // synchronize new blocks. This wakes any pending stream polls so they can make
            // progress again.
            if let Some(waker) = self.waker.take() {
                waker.wake()
            }
        }
    }

    pub fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<attestor_primitives::Height>,
    ) {
        self.attestation_interval = interval_new;
    }

    pub async fn note_attestation_chain_reversion(&mut self, info: stream_util::AttestationInfo) {
        use stream_util::ChainData as _;
        *self = self.reset(info).await;
    }

    /// Generates an attestation with no previous digest.
    pub fn generate_attestation_genesis(
        &self,
        stream_util::RootInfo { height, root, hash }: stream_util::RootInfo,
    ) -> Attestation {
        self.sign_attestation(
            attestor_primitives::AttestationData::new(self.chain_key, height, hash, root, None),
            Default::default(),
        )
    }

    fn generate_attestation(&self, target: attestor_primitives::Height) -> Attestation {
        assert!(!self.cache.is_empty(), "Empty root cache");

        let start = *self.computed.start();
        let end = *self.computed.end();

        assert!(target > start, "{target} > {start}",);
        assert!(target <= end, "{target} <= {end}",);

        let block_first = self.cache.first().unwrap().height;
        assert!(
            target >= block_first,
            "Invalid root cache start: {target} >= {block_first}"
        );

        let block_last = self.cache.last().unwrap().height;
        assert!(
            target <= block_last,
            "Invalid root cache stop: {target} <= {block_last}"
        );

        let index = target as usize - block_first as usize;

        let stream_util::RootInfo { height, root, hash } = self.cache[index];

        assert_eq!(height, target, "Attestation height mismatch");

        let blocks = self.cache[0..index].iter().fold(
            Vec::<attestor_primitives::block::BlockSerializable>::with_capacity(index),
            |mut acc, stream_util::RootInfo { height, root, .. }| {
                let digest_prev = acc
                    .last()
                    .map(|block| block.digest)
                    .unwrap_or(self.attestation_prev.digest);
                let block = attestor_primitives::block::Block::new_from_prev_digest(
                    *height,
                    *root,
                    digest_prev,
                );

                acc.push(block.into());
                acc
            },
        );

        assert_eq!(
            blocks.len(),
            (target - self.attestation_prev.height - 1) as usize,
            "Invalid continuity proof length"
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
        let attestor = self.signer.attestor_id();
        let message = attestation_data.serialize();
        let signature = sp_core::sr25519::Signature::from_raw(self.signer.sign(&message).0);
        let signature_bls = attestor_primitives::bls::WrapEncode(self.bls_key.sign(message));

        attestor_primitives::Attestation {
            attestation_data,
            attestor,
            signature,
            signature_bls,
            continuity_proof,
        }
    }

    /// Checks if the root cache contains all blocks up to the chain tip or the max catchup. If
    /// this is not the case we should keep processing new roots.
    fn missing_roots(&self) -> bool {
        self.cache
            .last()
            .map(|info| info.height)
            .unwrap_or_default()
            < self
                .computed
                .start()
                .saturating_add(self.max_catchup.get())
                .min(self.tip)
    }

    /// Makes sure the max catchup bound is respected.
    ///
    /// Note that if [`max_catchup`] is 500 for example then up to 501 roots may be stored in
    /// cache. This is because the attestation stream may start at block 0 and to ensure the
    /// highest cached root can always be a multiple of the max catchup.
    ///
    /// [`max_catchup`]: Self::max_catchup
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
            if self.cursor > *self.computed.start() {
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
                self.cursor = self.cursor.saturating_sub(self.attestation_interval.get());
                return std::task::Poll::Ready(Some(Ok(attestation)));
            }

            // Backpressure, limit the max number of roots which can be processed into a single
            // attestation
            if !self.fetching && self.cache.len() > self.max_catchup.get() as usize {
                self.waker = Some(cx.waker().clone());
                return std::task::Poll::Pending;
            }

            // We don't want the check above to be returning pending before we have finished
            // fetching roots.
            self.fetching = true;

            let next = self
                .computed
                .end()
                .to_owned()
                .saturating_add(self.attestation_interval.get());

            // Chain tip and roots are polled concurrently until a new attestation can be produced
            while self.tip < next || self.missing_roots() {
                let mut progress = false;

                if self.has_space_left() {
                    match self.stream_roots.poll_next_unpin(cx) {
                        std::task::Poll::Ready(Some(info)) => {
                            // Skip roots which are behind finality
                            if info.height > *self.computed.start() {
                                self.cache.push(info);
                            }
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
                        self.tip = tip - (tip % self.attestation_interval.get());
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
                .computed
                .start()
                .saturating_add(self.max_catchup.get())
                .min(self.tip);

            // We only update the state of attestation production after all roots have been cached
            // to avoid new attestations being generated mid-catchup.
            self.computed = *self.computed.end()..=stop;
            self.cursor = stop;
        }
    }
}

impl stream_util::ChainData<Result<Attestation, Error>> for StreamAttestation {
    async fn reset(&self, info: stream_util::AttestationInfo) -> Self {
        let config = ConfigBuilder::new()
            .with_signer(self.signer.clone())
            .with_chain_key(self.chain_key)
            .with_bls_key(self.bls_key.clone())
            .with_stream_roots(self.stream_roots.reset(info).await)
            .with_stream_tip(self.stream_tip.reset(info).await)
            .with_attestation_interval(self.attestation_interval)
            .with_attestation_prev(info)
            .with_max_catchup(self.max_catchup)
            .build();

        Self::new(config)
    }
}
