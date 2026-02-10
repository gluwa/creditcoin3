//! # Attestation Generation
//!
//! > _"Though this be madness, there is method in’t"_
//! > Polonius, Hamnet Act II scene 2
//!
//! This module  is responsible for the generation and catchup of new attestations from a source
//! chain. A lot of effort has been put into ensuring this follows the most optimal path to
//! consensus possible while maintaining the following properties:
//!
//! - **Fast finality:** new source chain blocks should be attested to as fast as possible.
//!
//! - **Redundancy:** failure to submit an attestation should not stall the network.
//!
//! - **Liveness:** changes in consensus should be gossiped rapidly.
//!
//! - **Efficiency:** catching up to many attestations at a time should not be any less performant
//!   than if we were to produce them one by one.
//!
//! This is implemented as a rather complex non-linear stream of attestations with makes heavy use
//! of caching to avoid duplicate computation. Getting this to work has taken much time and most of
//! my sanity, so please do be very careful if you have to update this, and make sure that the
//! following invariants hold at all times:
//!
//! 1. Attestors can NEVER produce more than the catchup limit number of attestations at a time.
//!
//! 2. Attestors can NEVER re-generate a past attestation.
//!
//! 3. Attestations MIGHT have to be pruned before they are propagated if a higher attestation
//!    finalizes first.
//!
//! 4. Attestations MIGHT finalize out of order despite being submitted in order due to network
//!    issues.
//!
//! 5. Source chain blocks MIGHT be produced out-of-order by whatever RPC is being used.
//!
//! 6. The attestation interval, checkpoint interval and catchup limit MIGHT change midway during
//!    catchup, and this must not break any of the other invariants.
//!
//! The code bellow has been written in such a way that if any of these invariants are violated
//! THEN THE ATTESTOR WILL CRASH. There is no recovery from an invalid state: the best we can do is
//! not to propagate it. This also makes it easier to detect bugs during testing: if something
//! isn't working, you will know.
//!
//! ## Catchup Strategy
//!
//! A key insight is that it doesn't really make sense to commit to anything else than the latest
//! attestation in ideal network conditions (which is most of the time). We still need to generate
//! past attestations as a fallback to handle adversarial conditions: it is much harder to censor
//! _many_ voting points than it is to sensor just a few.
//!
//! To avoid DOSing the execution chain, catchup is bounded to a multiple of the checkpoint
//! interval _in blocks_, set in [`MAX_CATCHUP`]. This is referred to as the **catchup limit**. For
//! example, if the attestation interval is 10 blocks, the checkpoint interval every 10
//! attestations, and the max catchup is 5, then the catchup limit will be 500 and the attestor will
//! be generating attestations of up to 500 blocks long.
//!
//! Attestations are produced _backwards_ from the highest possible source chain block which falls
//! within the catchup limit. For example, if the latest source chain block is 1200, but the latest
//! execution chain attestation height is only 500, then attestor will generate all attestations
//! between 500 and 1000, starting from 1000 (so 1000, 990, 980, 970... 510). Essentially, the
//! attestor "fills in the gap" in the attestation chain from top to bottom.
//!
//! This is done to ensure that, on average, the highest attestation will always reach quorum
//! amongst attestors _first_, while still allowing older attestations to be considered as a
//! backup. This way, we enforce a loose ordering of attestation submission based on the time it
//! takes to generate and propagate votes. This is not a constant, and due to this the range of
//! attestations being generated tends to fluctuate, though generally the attestor does a good job
//! of always voting on the highest point.
//!
//! ## Caching
//!
//! Since we generate attestations _backwards_ from the highest possible point in the source chain,
//! caching becomes trivial. Blocks and continuity proofs are fetched and computed only _once_ when
//! generating the first attestation in the catchup range and reused as much as possible. This is
//! handled by [`CacheRoots`] and [`CacheContinuity`] respectively.
//!
//! ## Point for improvement
//!
//! The current attestation stream does not support re-generating attestations. This can cause
//! invalidations during catchup, and some attestations to be dropped if the source chain is
//! finalizing faster than the runtime can keep up.
//!
//! The continuity proof is also gossiped as part of the P2P layer, which poses an issue on large
//! continuity proofs. This seems redundant as those values should be known locally anyways.
//!
//! [`MAX_CATCHUP`]: common::constants::MAX_CATCHUP

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

// ----------------------------------------- [ Stream ] ---------------------------------------- //

pub struct StreamAttestation {
    continuity: CacheContinuity,
    bls_key: bls_signatures::PrivateKey,

    block_n: common::types::Height,
    block_start: common::types::Height,
    block_stop: common::types::Height,
    block_head: common::types::Height,

    cc3: cc_client::Client,
    eth: eth::Client,
    stream: alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>,

    chain_key: attestor_primitives::ChainKey,
    interval_attestation: std::num::NonZero<common::types::Height>,
    interval_checkpoint: std::num::NonZero<common::types::Height>,

    waker: Option<std::task::Waker>,
    stop: bool,
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
    boundary: common::types::Height,
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

        let catchup_limit = checkpoint_in_blocks
            .saturating_mul(common::constants::MAX_CATCHUP)
            .get();

        let continuity = CacheContinuity::new(
            checkpoint_in_blocks,
            config.start_height,
            config.start_digest,
        );

        let mut stream = config
            .eth
            .subscribe()
            .await
            .context("Failed to initialize source chain connection")?;
        let next = stream
            .next()
            .await
            .context("Unexpected end of stream")?
            .number
            .saturating_sub(common::constants::ATTESTATION_FINALIZATION_LAG);

        let interval_attestation = config.interval_attestation.get();

        let block_head = next - (next % interval_attestation);
        let block_n = block_head.min(catchup_limit.saturating_add(config.start_height));
        let block_stop = config.start_height;
        let block_start = block_n;

        anyhow::Ok(Self {
            continuity,
            bls_key: config.bls_key,

            block_n,
            block_start,
            block_stop,
            block_head,

            cc3: config.cc3,
            eth: config.eth,
            stream,

            chain_key: config.chain_key,
            interval_attestation: config.interval_attestation,
            interval_checkpoint: config.interval_checkpoint,

            waker: None,
            stop: false,
        })
    }

    #[tracing::instrument(skip_all, fields(height = block_stop))]
    pub async fn generate_attestation(
        &mut self,
        Permit(block_stop): Permit,
    ) -> Result<common::types::Attestation, Error> {
        tracing::debug!("Generating attestation");

        self.continuity.update(&mut self.eth, block_stop).await?;

        assert!(
            !self.continuity.cache.is_empty(),
            "Cache should not be empty after an update"
        );

        let block_first = self.continuity.cache.first().unwrap().block_number;
        assert!(block_stop >= block_first, "{block_stop} >= {block_first}");

        let block_last = self.continuity.cache.last().unwrap().block_number;
        assert!(block_stop <= block_last, "{block_stop} <= {block_last}");

        let block_index = block_stop as usize - block_first as usize;
        let RootInfo { ref block, root } = self.continuity.roots.cache[block_index];

        let continuity_proof =
            attestor_primitives::attestation_fragment::AttestationFragmentSerializable {
                blocks: self.continuity.cache[0..block_index].as_ref().to_vec(),
            };

        assert_eq!(
            block.number(),
            block_stop,
            "Misstached block height in cache"
        );

        tracing::debug!(
            len = continuity_proof.blocks.len(),
            start = continuity_proof
                .blocks
                .first()
                .map(|block| block.block_number),
            stop = continuity_proof
                .blocks
                .last()
                .map(|block| block.block_number),
            "Generated proof"
        );

        let attestation_data = common::types::AttestationData::new(
            self.chain_key,
            block_stop,
            attestor_primitives::Digest::from(*block.hash()),
            root,
            continuity_proof.head().map(|head| head.digest),
        );

        let attestation = self.sign_attestation(attestation_data, continuity_proof);

        Ok(attestation)
    }

    #[tracing::instrument(skip_all, level = "debug")]
    pub async fn generate_attestation_genesis(
        &mut self,
    ) -> Result<common::types::Attestation, Error> {
        assert!(self.continuity.cache.is_empty());
        assert!(self.continuity.roots.cache.is_empty());

        let start_height = self.continuity.roots.boundary;

        tracing::debug!(start_height, "Generating genesis attestation");

        self.continuity
            .roots
            .update(&mut self.eth, start_height)
            .await?;

        assert!(
            !self.continuity.roots.cache.is_empty(),
            "Cache should not be empty after an update"
        );

        let RootInfo { ref block, root } = self.continuity.roots.cache[0];
        let prev_digest = None;

        let attestation_data = common::types::AttestationData::new(
            self.chain_key,
            start_height,
            attestor_primitives::Digest::from(*block.hash()),
            root,
            prev_digest,
        );

        Ok(self.sign_attestation(attestation_data, Default::default()))
    }

    pub fn block_highest(&self) -> common::types::Height {
        self.block_head
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
            .saturating_mul(self.interval_checkpoint);
        let size_new = checkpoint_in_blocks
            .saturating_mul(common::constants::MAX_CATCHUP)
            .saturating_add(1)
            .try_into()
            .unwrap();

        if self.continuity.max_size < size_new {
            let additional = size_new.get() - self.continuity.max_size.get();
            self.continuity.cache.reserve(additional);
            self.continuity.roots.cache.reserve(additional);
        } else {
            // NOTE: interval updates happen infrequently enough and should be small enough that
            // they should not be a concern for RAM usage, hence we do not bother with shrinking
            // cache allocation on a smaller interval
        }

        self.continuity.max_size = size_new;
        self.continuity.roots.max_size = size_new;
    }

    async fn block_next(&mut self) -> Option<Result<common::types::Height, Error>> {
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

        // Attestation interval might have changed in between, always align the current block to it
        let block_n = self.block_n - (self.block_n % self.interval_attestation.get());

        // We have not finished producing attestations backwards
        if block_n > self.block_stop {
            let permit = Permit(block_n);
            self.block_n = block_n.saturating_sub(self.interval_attestation.get());

            return std::task::Poll::Ready(Some(Ok(permit)));
        }

        // We have produced the lowest possible attestation, now we check to see if we can produce
        // attestations at a greater height while still respecting the catchup limit

        let checkpoint_in_blocks = self
            .interval_attestation
            .saturating_mul(self.interval_checkpoint);
        let catchup_limit = checkpoint_in_blocks
            .saturating_mul(common::constants::MAX_CATCHUP)
            .get();

        // The catchup limit is ALWAYS based on the cache size. It is not reliable to refer to
        // `self.block_start` and `self.block_stop` as there are multiple events which might purge
        // the cache which are not reflected in those variables. The catchup limit is a guarantee on
        // the max size of the cache, and so we use that as a reference.
        let cache_size = self.continuity.cache.len() as u64;

        if cache_size < catchup_limit {
            self.stop = false;

            tracing::debug!(
                block_n,
                block_stop = self.block_stop,
                block_start = self.block_start,
                catchup_limit,
                "Fetching next catchup section"
            );

            // `self.block_stop` is updated as new attestations finalize so that we do not generate
            // attestations which have already reached finality on the execution chain. Because of
            // this, it can be that `self.block_stop > self.block_start` if an attestor is lagging
            // behind.
            let block_stop = self.block_stop.max(self.block_start);

            // We do not trust the RPC we are connected to to return block in order. Keep looping
            // until we get the next best height.
            while self.block_head <= block_stop {
                //
                // Fun fact: `eth.get_last_block` does not return the latest block despite calling
                // `get_block_number`! :D
                //
                // https://knowyourmeme.com/memes/hide-the-pain-harold
                let block_n = match std::task::ready!(std::pin::pin!(self.block_next()).poll(cx)) {
                    Some(Ok(block_n)) => block_n,
                    Some(Err(err)) => return std::task::Poll::Ready(Some(Err(err))),
                    None => return std::task::Poll::Ready(Some(Err(Error::Interrupt))),
                };

                self.block_head =
                    block_n.saturating_sub(common::constants::ATTESTATION_FINALIZATION_LAG);
                self.block_head -= self.block_head % self.interval_attestation.get();
            }

            self.block_n = self
                .block_head
                .min(self.block_stop.saturating_add(catchup_limit - cache_size));
            self.block_stop = block_stop;
            self.block_start = self.block_n;

            // We do NOT loop on poll_next to keep it simple to execute. Instead we use the next
            // best height as the point to generate the next attestation and update the state
            // accordingly.
            let permit = Permit(self.block_n);
            self.block_n = self.block_n.saturating_sub(self.interval_attestation.get());

            tracing::debug!(
                block_n = self.block_n,
                blocl_start = self.block_start,
                block_stop = self.block_stop,
                block_head = self.block_head,
                "Updated catchup"
            );

            return std::task::Poll::Ready(Some(Ok(permit)));
        } else if !self.stop {
            self.stop = true;
            tracing::warn!(
                block_start = self.block_start,
                block_stop = self.block_stop,
                "🏃 Max catchup reached"
            );
        }

        // We don't really care about replacing a previous waker since we are executing this code
        // in a single-threaded asynchronous context using a custom Tokio runtime, and so we should
        // not be observing any contention on this future. This differs from the attestation pool,
        // which needs to keep a queue of past wakers and cannot override them else it risks
        // stalling other threads.
        self.waker.replace(cx.waker().clone());
        std::task::Poll::Pending
    }
}

impl CacheContinuity {
    pub fn new(
        checkpoint_in_blocks: std::num::NonZero<common::types::Height>,
        start_height: common::types::Height,
        start_digest: Option<attestor_primitives::Digest>,
    ) -> Self {
        let max_size: std::num::NonZeroUsize = checkpoint_in_blocks
            .saturating_mul(common::constants::MAX_CATCHUP)
            .saturating_add(1) // Inclusive
            .try_into()
            .unwrap();

        Self {
            cache: Vec::with_capacity(max_size.get()),
            prev_digest: start_digest.unwrap_or_default(),
            max_size,

            roots: CacheRoots::new(checkpoint_in_blocks, start_height),
        }
    }

    #[tracing::instrument(skip_all, fields(block_stop))]
    async fn update(
        &mut self,
        eth: &mut eth::Client,
        height_stop: common::types::Height,
    ) -> Result<(), Error> {
        self.roots.update(eth, height_stop).await?;

        let height_first = self
            .cache
            .first()
            .map(|info| info.block_number)
            .unwrap_or(self.roots.boundary);
        let height_last = self
            .cache
            .last()
            // FIXME: overflow breaks other invariants, saturating isn't enough
            .map(|info| info.block_number + 1)
            .unwrap_or(self.roots.boundary);

        if height_stop < height_last
            || self
                .cache
                .len()
                .checked_add(height_stop as usize - height_last as usize)
                .is_none_or(|len_new| len_new > self.max_size.get())
        {
            return Ok(());
        }

        tracing::debug!(
            height_last,
            height_stop,
            start_height = self.roots.boundary,
            "Computing continuity proof"
        );

        tracing::info!(
            "[({height_first})/{height_last}:{height_stop}]: {:?}",
            self.cache
                .iter()
                .map(|block| block.block_number)
                .collect::<Vec<_>>()
        );

        let size_fragment = height_stop as usize - height_last as usize + 1;
        let size_roots = self.roots.cache.len();

        assert!(
            size_fragment <= size_roots,
            "{size_fragment} < {size_roots}"
        );

        let roots_start = self.roots.cache[0].block.number();

        assert!(roots_start <= height_last, "{roots_start} <= {height_last}");

        let index_start = (height_last - roots_start) as usize;
        let index_stop = index_start + size_fragment;

        tracing::debug!(
            index_start,
            index_stop,
            start_height = self.roots.boundary,
            "Computing missing segments"
        );

        for RootInfo { block, root } in &self.roots.cache[index_start..index_stop] {
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

        tracing::info!(
            "[({height_first})/{height_last}:{height_stop}]: {:?}",
            self.cache
                .iter()
                .map(|block| block.block_number)
                .collect::<Vec<_>>()
        );

        let len_cache = self.cache.len();
        let max_size = self.max_size.get();

        assert!(
            len_cache <= max_size,
            "Invalid continuity cache size: {len_cache} <= {max_size}"
        );

        Ok(())
    }
}

impl CacheRoots {
    pub fn new(
        checkpoint_in_blocks: std::num::NonZero<common::types::Height>,
        start_height: common::types::Height,
    ) -> Self {
        let max_size: std::num::NonZeroUsize = checkpoint_in_blocks
            .saturating_mul(common::constants::MAX_CATCHUP)
            .saturating_add(1) // Inclusive
            .try_into()
            .unwrap();

        Self {
            cache: Vec::with_capacity(max_size.get()),
            max_size,
            boundary: start_height,
        }
    }

    #[tracing::instrument(skip_all)]
    async fn update(
        &mut self,
        eth: &mut eth::Client,
        height_stop: common::types::Height,
    ) -> Result<(), Error> {
        use futures::FutureExt as _;
        use rayon::iter::IntoParallelIterator as _;
        use rayon::iter::ParallelExtend as _;
        use rayon::iter::ParallelIterator as _;

        let height_first = self
            .cache
            .first()
            .map(|info| info.block.number())
            .unwrap_or(self.boundary);
        let height_last = self
            .cache
            .last()
            // FIXME: overflow breaks other invariants, saturating isn't enough
            .map(|info| info.block.number() + 1)
            .unwrap_or(self.boundary);

        if height_stop < height_last {
            if self
                .cache
                .len()
                .checked_sub(height_stop as usize - height_last as usize)
                .is_none_or(|len_new| len_new <= self.max_size.get())
            {
                tracing::info!(
                    height_last,
                    height_stop,
                    start_height = self.boundary,
                    "🎯 Cache hit"
                );
            }
            return Ok(());
        }

        tracing::info!(
            height_last,
            height_stop,
            start_height = self.boundary,
            "🎯 Cache miss"
        );
        tracing::debug!(
            height_last,
            height_stop,
            start_height = self.boundary,
            "Computing digests"
        );

        let encoding = ccnext_abi_encoding::common::EncodingVersion::V1;
        let iter = (height_last..=height_stop).map(|h| {
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

        let len_cache = self.cache.len();
        let max_size = self.max_size.get();

        tracing::info!(
            "[({height_first})/{height_last}:{height_stop}]: {:?}",
            self.cache
                .iter()
                .map(|info| info.block.number())
                .collect::<Vec<_>>()
        );

        assert!(
            len_cache <= max_size,
            "From {} - Invalid digest cache size: {len_cache} <= {max_size} [({height_first})/{height_last}:{height_stop}]: {:?}",
            self.boundary,
            self.cache.iter().map(|info| info.block.number()).collect::<Vec<_>>()
        );

        assert!(
            !self.cache.is_empty(),
            "Cache cannot be empty after an update [({height_first}){height_last}:{height_stop}]"
        );

        Ok(())
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl crate::events::EventAttestationFinalizationAsync for StreamAttestation {
    type Error = ();

    #[tracing::instrument(skip_all, fields(height = info.height, digest = %info.digest))]
    async fn note_attestation_finalization_async(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Self::Error> {
        use crate::events::EventAttestationFinalization as _;

        let interval_attestation = self.interval_attestation.get();
        let height = info.height - (info.height % interval_attestation);

        tracing::debug!(
            block_stop = self.block_stop,
            height,
            "Updating attestation stream"
        );

        if self.block_stop < height {
            self.block_stop = height;
            self.continuity.roots.boundary = height;
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

        tracing::debug!("Updating continuity cache");

        self.cache.clear();

        if info.height >= self.roots.boundary {
            self.prev_digest = info.digest;
        }

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
            let height_first = self.cache.first().unwrap().block.number();

            tracing::info!(
                "[{height_first}:{}]: {:?}",
                info.height,
                self.cache
                    .iter()
                    .map(|info| info.block.number())
                    .collect::<Vec<_>>()
            );

            if info.height >= height_first {
                let index_stop =
                    (info.height as usize - height_first as usize + 1).min(self.cache.len());

                let removed_last = self
                    .cache
                    .drain(0..index_stop)
                    .next_back()
                    .unwrap()
                    .block
                    .number();

                tracing::info!(
                    "[{height_first}:{}]: {:?}",
                    info.height,
                    self.cache
                        .iter()
                        .map(|info| info.block.number())
                        .collect::<Vec<_>>()
                );

                assert_eq!(removed_last, info.height);

                self.boundary = info.height.saturating_add(1);
            }
        } else {
            self.boundary = info.height.saturating_add(1);
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
