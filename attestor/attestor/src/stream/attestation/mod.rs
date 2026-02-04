use crate::prelude::*;

mod error;

pub use error::Error;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    eth: eth::Client,
    bls_key: bls_signatures::PrivateKey,

    interval_attestation: std::num::NonZero<common::types::Height>,
    interval_checkpoint: std::num::NonZero<common::types::Height>,

    chain_key: attestor_primitives::ChainKey,
    start_height: common::types::Height,
    start_digest: Option<attestor_primitives::Digest>,
}

// ----------------------------------------- [ Types ] ----------------------------------------- //

pub struct StreamAttestation {
    continuity: CacheContinuity,
    bls_key: bls_signatures::PrivateKey,

    block_start: common::types::Height,
    block_latest: common::types::Height,
    block_current: common::types::Height,
    block_limit: common::types::Height,

    cc3: cc_client::Client,
    eth: eth::Client,
    stream: alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>,

    chain_key: attestor_primitives::ChainKey,
    interval_attestation: std::num::NonZero<common::types::Height>,
    interval_checkpoint: std::num::NonZero<common::types::Height>,

    waker: Option<std::task::Waker>,
}

pub struct Permit(common::types::Height);

struct CacheContinuity {
    cache: Vec<attestor_primitives::block::BlockSerializable>,
    prev_digest: attestor_primitives::Digest,
    max_size: std::num::NonZeroUsize,

    roots: CacheRoots,
}

struct CacheRoots {
    cache: Vec<RootInfo>,
    max_size: std::num::NonZeroUsize,
    start_height: common::types::Height,
}

#[derive(Debug, Clone)]
struct RootInfo {
    block: eth::OrderedBlock,
    root: attestor_primitives::Digest,
}

impl StreamAttestation {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        use anyhow::Context as _;
        use futures::StreamExt as _;

        let checkpoint_in_blocks = config
            .interval_attestation
            .saturating_mul(config.interval_checkpoint);

        let continuity = CacheContinuity::new(
            checkpoint_in_blocks.try_into().unwrap(),
            config.start_height,
            config.start_digest,
        );

        let mut stream = config
            .eth
            .subscribe()
            .await
            .context("Failed to initialize source chain connection")?;
        let block_next = stream
            .next()
            .await
            .context("Unexpected end of stream")?
            .number
            .saturating_sub(common::constants::ATTESTATION_FINALIZATION_LAG);

        let interval_attestation = config.interval_attestation.get() as common::types::Height;

        let block_start = config.start_height;
        let block_latest = block_next - (block_next % interval_attestation);
        let block_current = block_start
            .saturating_add(checkpoint_in_blocks.get())
            .min(block_latest);
        let block_limit = block_current;

        anyhow::Ok(Self {
            continuity,
            bls_key: config.bls_key,

            block_start,
            block_latest,
            block_current,
            block_limit,

            cc3: config.cc3,
            eth: config.eth,
            stream,

            chain_key: config.chain_key,
            interval_attestation: config.interval_attestation,
            interval_checkpoint: config.interval_checkpoint,

            waker: None,
        })
    }

    pub async fn generate_attestation(
        &mut self,
        Permit(block_stop): Permit,
    ) -> Result<common::types::Attestation, Error> {
        self.continuity.update(&mut self.eth, block_stop).await?;

        assert!(
            !self.continuity.cache.is_empty(),
            "Cache should not be empty after an update"
        );

        let block_first = self.continuity.cache.first().unwrap().block_number;
        assert!(block_stop >= block_first, "{block_stop} >= {block_first}");

        let block_last = self.continuity.cache.last().unwrap().block_number;
        assert!(block_stop <= block_last, "{block_stop} <= {block_last}");

        let block_index = block_stop - block_first;
        let RootInfo { ref block, root } = self.continuity.roots.cache[block_index as usize];

        assert_eq!(
            block.number(),
            block_stop,
            "Misstached block height in cache"
        );

        let attestation_data = common::types::AttestationData::new(
            self.chain_key,
            block.number(),
            attestor_primitives::Digest::from(*block.hash()),
            root,
            Some(self.continuity.prev_digest),
        );

        let attestation = self.sign_attestation(attestation_data, Default::default());

        Ok(attestation)
    }

    #[tracing::instrument(skip_all, level = "debug")]
    pub async fn generate_attestation_genesis(
        &mut self,
    ) -> Result<common::types::Attestation, Error> {
        assert!(self.continuity.cache.is_empty());
        assert!(self.continuity.roots.cache.is_empty());

        let start_height = self.continuity.roots.start_height;

        self.continuity
            .roots
            .update(&mut self.eth, start_height)
            .await?;

        assert!(!self.continuity.roots.cache.is_empty());

        let RootInfo { ref block, root } = self.continuity.roots.cache[0];
        let prev_digest = None;

        let attestation_data = common::types::AttestationData::new(
            self.chain_key,
            self.block_start,
            attestor_primitives::Digest::from(*block.hash()),
            root,
            prev_digest,
        );

        Ok(self.sign_attestation(attestation_data, Default::default()))
    }

    pub fn block_highest(&self) -> common::types::Height {
        self.block_limit
    }

    fn sign_attestation(
        &mut self,
        attestation_data: attestor_primitives::AttestationData<attestor_primitives::Digest>,
        continuity_proof: attestor_primitives::attestation_fragment::AttestationFragmentSerializable,
    ) -> common::types::Attestation {
        let attestor = self.cc3.get_attestor_id();
        let message = attestation_data.serialize();
        let signature = sp_core::sr25519::Signature::from_raw(self.cc3.sign(&message).0);
        let signature_bls = attestor_primitives::bls::WrapEncode(self.bls_key.sign(message));

        common::types::Attestation {
            attestation_data,
            attestor,
            signature,
            signature_bls,
            continuity_proof,
        }
    }

    fn update_interval(&mut self) {
        let checkpoint_in_blocks = self
            .interval_attestation
            .saturating_mul(self.interval_checkpoint)
            .try_into()
            .unwrap();

        if self.continuity.max_size < checkpoint_in_blocks {
            let additional = checkpoint_in_blocks.get() - self.continuity.max_size.get();
            self.continuity.cache.reserve(additional);

            let additional = checkpoint_in_blocks.get() * 2 - self.continuity.roots.max_size.get();
            self.continuity.roots.cache.reserve(additional);
        } else {
            // NOTE: interval updates happen infrequently enough and should be small enough that
            // they should not be a concern for RAM usage, hence we do not bother with shrinking
            // cache allocation on a smaller interval
        }

        self.continuity.max_size = checkpoint_in_blocks;
        self.continuity.roots.max_size = checkpoint_in_blocks;
    }

    async fn next_block(&mut self) -> Option<Result<common::types::Height, Error>> {
        use futures::stream::StreamExt as _;

        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        loop {
            match self.stream.next().await {
                Some(block) => break Some(Ok(block.number)),
                None => match self.eth.subscribe().await {
                    Ok(sub) => self.stream = sub,
                    Err(err) => {
                        attempt += 1;

                        tracing::debug!(
                            attempt,
                            MAX_ATTEMPTS,
                            "Failed to reconnect to eth, retrying..."
                        );

                        if attempt >= MAX_ATTEMPTS {
                            break Some(Err(Error::Eth(err)));
                        }
                    }
                },
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => break None
            }

            delay = (delay * 2).min(DELAY_MAX);
        }
    }
}

impl futures::Stream for StreamAttestation {
    type Item = Result<Permit, Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use std::future::Future as _;

        let interval_attestation = self.interval_attestation.get();
        let interval_checkpoint = self.interval_checkpoint.get();

        let block_current = self.block_current - (self.block_current % interval_attestation);
        if block_current > self.block_start {
            self.block_current = block_current.saturating_sub(interval_attestation);
            return std::task::Poll::Ready(Some(Ok(Permit(block_current))));
        }

        let checkpoint_in_blocks = interval_attestation * interval_checkpoint;
        if self.block_limit - self.block_current >= checkpoint_in_blocks {
            assert!(self.waker.is_none());
            self.waker.replace(cx.waker().clone());
        }

        let mut block_next = self.block_latest;
        while block_next <= self.block_latest {
            let fut = std::pin::pin!(self.next_block());
            let block_n = match std::task::ready!(fut.poll(cx)) {
                Some(Ok(block_n)) => block_n,
                Some(Err(err)) => return std::task::Poll::Ready(Some(Err(err))),
                None => return std::task::Poll::Ready(Some(Err(Error::Interrupt))),
            };

            block_next = block_n.saturating_sub(common::constants::ATTESTATION_FINALIZATION_LAG);
            block_next -= block_next % interval_attestation;
        }

        let block_target = self
            .block_start
            .saturating_add(checkpoint_in_blocks)
            .min(block_next);

        self.block_start = self.block_latest;
        self.block_latest = block_next;
        self.block_current = block_target.saturating_sub(interval_attestation);
        self.block_limit = self.block_current;

        std::task::Poll::Ready(Some(Ok(Permit(block_target))))
    }
}

impl CacheContinuity {
    pub fn new(
        checkpoint_in_blocks: std::num::NonZeroUsize,
        start_height: common::types::Height,
        start_digest: Option<attestor_primitives::Digest>,
    ) -> Self {
        Self {
            cache: Vec::with_capacity(checkpoint_in_blocks.get()),
            prev_digest: start_digest.unwrap_or_default(),
            max_size: checkpoint_in_blocks,

            roots: CacheRoots::new(checkpoint_in_blocks, start_height),
        }
    }

    async fn update(
        &mut self,
        eth: &mut eth::Client,
        block_stop: common::types::Height,
    ) -> Result<(), Error> {
        self.roots.update(eth, block_stop).await?;

        let block_start = self
            .cache
            .last()
            .map(|info| info.block_number)
            .unwrap_or(self.roots.start_height);

        if block_stop < block_start
            || self
                .cache
                .len()
                .checked_add(block_stop as usize - block_start as usize)
                .is_none_or(|len_new| len_new > self.max_size.get())
        {
            return Ok(());
        }

        let size_fragment = (block_stop - block_start).saturating_add(1) as usize;
        let size_roots = self.roots.cache.len();

        assert!(size_fragment < size_roots, "{size_fragment} < {size_roots}");

        let roots_start = self.roots.cache[0].block.number();

        assert!(roots_start <= block_start, "{roots_start} <= {block_start}");

        let index_start = (block_start - roots_start) as usize;
        let index_stop = index_start + size_fragment;

        for RootInfo { block, root } in &self.roots.cache[index_start..=index_stop] {
            let prev_digest = self
                .cache
                .last()
                .map(|block| block.digest)
                .unwrap_or(self.prev_digest);
            let block = attestor_primitives::block::Block::new_from_prev_digest(
                block.number(),
                *root,
                prev_digest,
            );

            self.cache
                .push(attestor_primitives::block::BlockSerializable::from(block));
        }

        Ok(())
    }
}

impl CacheRoots {
    pub fn new(
        checkpoint_in_blocks: std::num::NonZeroUsize,
        start_height: common::types::Height,
    ) -> Self {
        Self {
            cache: Vec::with_capacity(checkpoint_in_blocks.get() * 2),
            max_size: checkpoint_in_blocks,
            start_height,
        }
    }

    #[tracing::instrument(skip_all)]
    async fn update(
        &mut self,
        eth: &mut eth::Client,
        block_stop: common::types::Height,
    ) -> Result<(), Error> {
        use futures::FutureExt as _;
        use rayon::iter::IntoParallelIterator as _;
        use rayon::iter::ParallelExtend as _;
        use rayon::iter::ParallelIterator as _;

        let block_start = self
            .cache
            .last()
            .map(|info| info.block.number())
            .unwrap_or(self.start_height);

        if block_stop < block_start
            || self
                .cache
                .len()
                .checked_sub(block_stop as usize - block_start as usize)
                .is_none_or(|len_new| len_new > self.max_size.get())
        {
            tracing::info!(block_start, block_stop, "🎯 Cache hit");
            return Ok(());
        }

        tracing::info!(block_start, block_stop, "🎯 Cache miss");

        let encoding = ccnext_abi_encoding::common::EncodingVersion::V1;
        let iter = (block_start..=block_stop).map(|h| {
            eth.get_block(h, encoding).map(|opt| {
                opt.ok_or(Error::Interrupt)
                    .and_then(|res| res.map_err(Error::Eth))
            })
        });
        let blocks = futures::future::try_join_all(iter).await?;

        self.cache
            .par_extend(blocks.into_par_iter().map(|block| RootInfo {
                root: eth::simple_merkle_tree(&block).root(),
                block,
            }));

        assert!(
            self.cache.len() < self.max_size.get(),
            "Invalid digest cache size"
        );

        assert!(
            !self.cache.is_empty(),
            "Cache cannot be empty after an update"
        );

        Ok(())
    }
}

impl crate::events::EventAttestationFinalizationAsync for StreamAttestation {
    type Error = ();

    async fn note_attestation_finalization_async(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Self::Error> {
        use crate::events::EventAttestationFinalization as _;

        let interval_attestation = self.interval_attestation.get();
        let height = info.height + interval_attestation - (info.height % interval_attestation);

        if self.block_current < height {
            self.block_current = height;
        }

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }

        self.continuity
            .note_attestation_finalization(info)
            .expect("Infallible");

        Ok(())
    }
}
impl crate::events::EventAttestationFinalization for StreamAttestation {}

impl crate::events::EventAttestationFinalizationAsync for CacheContinuity {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization_async(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Self::Error> {
        use crate::events::EventAttestationFinalization as _;

        self.cache.clear();

        self.roots
            .note_attestation_finalization(info)
            .expect("Infallible");

        Ok(())
    }
}
impl crate::events::EventAttestationFinalization for CacheContinuity {}

impl crate::events::EventAttestationFinalizationAsync for CacheRoots {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization_async(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Self::Error> {
        if !self.cache.is_empty() {
            let block_first = self.cache.first().unwrap().block.number();
            let block_last = self.cache.last().unwrap().block.number();

            if info.height >= block_first && info.height <= block_last {
                let index_stop = info.height as usize - block_first as usize;
                let removed_last = self
                    .cache
                    .drain(0..=index_stop)
                    .next_back()
                    .unwrap()
                    .block
                    .number();

                assert_eq!(removed_last, info.height);
            }
        }

        Ok(())
    }
}
impl crate::events::EventAttestationFinalization for CacheRoots {}

impl crate::events::EventAttestationIntervalChangeAsync for StreamAttestation {
    type Error = std::convert::Infallible;

    async fn note_attestation_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: common::types::Height,
    ) -> Result<(), Self::Error> {
        self.interval_attestation = interval_new;
        self.update_interval();
        Ok(())
    }
}
impl crate::events::EventAttestationIntervalChange for StreamAttestation {}

impl crate::events::EventCheckpointIntervalChangeAsync for StreamAttestation {
    type Error = std::convert::Infallible;

    async fn note_checkpoint_interval_change_async(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: common::types::Height,
    ) -> Result<(), Self::Error> {
        self.interval_attestation = interval_new;
        self.update_interval();
        Ok(())
    }
}
impl crate::events::EventCheckpointIntervalChange for StreamAttestation {}
