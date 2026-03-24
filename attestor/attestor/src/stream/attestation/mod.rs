//! # Attestation Generation
//!
//! > _"Though this be madness, there is method in’t"_
//! >
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
//! of caching to avoid duplicate computation.
//!
//! ## Invariants
//!
//! > _"C makes it easy to shoot yourself in the foot; C++ makes it harder, but when you do it blows
//! > your whole leg off"_
//! >
//! > Bjarne Stroustrup
//!
//! Getting this to work has taken much time and most of my sanity, so please do be very careful if
//! you have to update this, and make sure that the following invariants hold at all times:
//!
//! 1. Attestors can **NEVER** produce more than the catchup limit number of attestations at a time.
//!
//! 2. Attestors can **NEVER** re-generate a past attestation. In the case of a chain reversion we
//!    can generate new attestations at the same height as past attestations which were reverted.
//!    This doesn't count as re-generating, since the roots and digests will be different.
//!
//! 3. Attestations **MIGHT** be dropped before they can be propagated if a higher attestation
//!    finalizes first.
//!
//! 4. Attestations **MIGHT** finalize out of order despite being submitted in order due to network
//!    issues.
//!
//! 5. Source chain blocks **MIGHT** be produced out-of-order by whatever RPC is being used.
//!
//! 6. The attestation interval, checkpoint interval and catchup limit **MIGHT** change midway
//!    during catchup, and this must not break any of the other invariants.
//!
//! The code bellow has been written in such a way that if any of these invariants are violated
//! **THEN THE ATTESTOR WILL CRASH**. There is no recovery from an invalid state: the best we can do
//! is not to propagate it. This also makes it easier to detect bugs during testing: if something
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
//! execution chain attestation height is only 500, then attestors will generate all attestations
//! between 500 and 1000, starting from 1000 (so 1000, 990, 980, 970... 510). Essentially, the
//! attestor "fills in the gap" in the attestation chain from top to bottom.
//!
//! This is done to ensure that, on average, the highest attestation will always reach quorum
//! amongst attestors _first_, while still allowing older attestations to be considered as a
//! backup. This way, we enforce a loose ordering of attestation submission based on the time it
//! takes to generate and propagate votes. This is not a constant, and due to this the range of
//! attestations being generated tends to fluctuate, though generally the attestor does a good job
//! at always submitting the highest attestation first.
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

#[derive(Debug, builder::Builder)]
pub struct Config {
    cc3: cc_client::Client,
    eth: eth::Client,
    bls_key: bls_signatures::PrivateKey,

    interval_attestation: std::num::NonZero<common::types::Height>,
    // The delay in blocks before a source chain block is considered mature enough to attest to.
    maturity_delay: common::types::Height,

    chain_key: attestor_primitives::ChainKey,
    start_height: common::types::Height,
    start_attestation: Option<common::types::AttestationInfo>,

    #[default(usc_abi_encoding::common::EncodingVersion::V1)]
    encoding_version: usc_abi_encoding::common::EncodingVersion,
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
    state: State,

    chain_key: attestor_primitives::ChainKey,
    interval_attestation: std::num::NonZero<common::types::Height>,
    maturity_delay: common::types::Height,

    waker: Option<std::task::Waker>,
    stop: bool,

    encoding_version: usc_abi_encoding::common::EncodingVersion,

    // Stored for recreating eth client and stream in the case of a chain reversion.
    // Why is this necessary? If the chain is reverted while the attestation stream
    // is in `State::Polling`, then we have no way of accessing the current eth client
    // and stream until polling is complete. Waiting would require the introduction
    // of complex and fragile `pending reversion` logic. Instead, we can safely drop
    // the future containing our old eth client and stream since whatever work was
    // being done is now invalid anyways.
    eth_transport: eth::ConnectionTransport,
}

pub struct Permit(common::types::Height);

pub struct CacheContinuity {
    cache: Vec<attestor_primitives::block::BlockSerializable>,
    prev_digest: attestor_primitives::Digest,

    roots: CacheRoots,
}

pub struct CacheRoots {
    cache: Vec<RootInfo>,
    max_size: std::num::NonZeroUsize,
    boundary: common::types::Height,
    encoding_version: usc_abi_encoding::common::EncodingVersion,
}

type NextBlockFut = dyn std::future::Future<Output = (State, Result<common::types::Height, Interrupt<Error>>)>
    + Send;

#[allow(clippy::large_enum_variant)]
enum State {
    Idle(
        Option<(
            eth::Client,
            alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>,
        )>,
    ),
    Polling(std::pin::Pin<Box<NextBlockFut>>),
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

        let continuity = CacheContinuity::new(
            config.start_height,
            config.start_attestation,
            config.encoding_version,
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
            .saturating_sub(config.maturity_delay);

        let interval_attestation = config.interval_attestation.get();

        let block_head = next - (next % interval_attestation);
        let block_n = block_head.min(
            common::constants::MAX_CATCHUP
                .get()
                .saturating_add(config.start_height),
        );
        let block_stop = config.start_height;
        let block_start = block_n;

        let eth_transport = config.eth.get_url()?;

        anyhow::Ok(Self {
            continuity,
            bls_key: config.bls_key,

            block_n,
            block_start,
            block_stop,
            block_head,

            cc3: config.cc3,
            state: State::Idle(Some((config.eth, stream))),

            chain_key: config.chain_key,
            interval_attestation: config.interval_attestation,
            maturity_delay: config.maturity_delay,

            waker: None,
            stop: false,

            encoding_version: config.encoding_version,

            eth_transport,
        })
    }

    #[tracing::instrument(skip_all, fields(height = block_stop))]
    pub async fn generate_attestation(
        &mut self,
        Permit(block_stop): Permit,
    ) -> Result<common::types::Attestation, Interrupt<Error>> {
        tracing::debug!("Generating attestation");

        let State::Idle(Some((eth, _))) = &mut self.state else {
            unreachable!();
        };

        self.continuity.update(eth, block_stop).await?;

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
    ) -> Result<common::types::Attestation, Interrupt<Error>> {
        assert!(self.continuity.cache.is_empty());
        assert!(self.continuity.roots.cache.is_empty());

        let start_height = self.continuity.roots.boundary;

        tracing::debug!(start_height, "Generating genesis attestation");

        let State::Idle(Some((eth, _))) = &mut self.state else {
            unreachable!();
        };

        self.continuity.roots.update(eth, start_height).await?;

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

    async fn block_next(
        eth: eth::Client,
        mut stream: alloy::pubsub::SubscriptionStream<alloy::rpc::types::Header>,
    ) -> (State, Result<common::types::Height, Interrupt<Error>>) {
        use futures::stream::StreamExt as _;

        const MAX_ATTEMPTS: usize = 5;
        const DELAY_BASE: u64 = 10;
        const DELAY_MAX: u64 = 60;

        let mut attempt = 0;
        let mut delay = DELAY_BASE;

        loop {
            match stream.next().await {
                Some(block) => break (State::Idle(Some((eth, stream))), Ok(block.number)),
                None => match eth.subscribe().await {
                    Ok(stream_new) => stream = stream_new,
                    Err(err) => {
                        attempt += 1;

                        tracing::debug!(
                            attempt,
                            MAX_ATTEMPTS,
                            "Failed to reconnect to eth, retrying..."
                        );

                        if attempt >= MAX_ATTEMPTS {
                            tracing::error!(error = %err, "⛔ Failed to reconnect to eth");
                        }
                    }
                },
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(delay))=> {},
                _ = tokio::signal::ctrl_c() => {
                    break (
                        State::Idle(Some((eth, stream))),
                        Err(Interrupt::Stop)
                    )
                },
            }

            delay = (delay * 2).min(DELAY_MAX);
        }
    }
}

impl futures::Stream for StreamAttestation {
    type Item = Result<Permit, Interrupt<Error>>;

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

        // The catchup limit is ALWAYS based on the cache size. It is not reliable to refer to
        // `self.block_start` and `self.block_stop` as there are multiple events which might purge
        // the cache which are not reflected in those variables. The catchup limit is a guarantee on
        // the max size of the cache, and so we use that as a reference.
        let cache_size = self.continuity.roots.cache.len() as u64;

        if cache_size < common::constants::MAX_CATCHUP.get() {
            self.stop = false;

            tracing::debug!(
                block_n,
                block_stop = self.block_stop,
                block_start = self.block_start,
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
                // Calls to `poll_next` must be cancellation-safe!
                //
                // This means that any progress made before returning `std::task::Poll::Pending`
                // should not be lost! When a future retuns `std::task::Poll::Pending`, it is
                // indicating to the async monitor that it cannot make any more progress in its
                // execution: here, this happens when reaching the catchup limit. Execution will be
                // resumed when the future is woken via a call to `std::task::Waker::wake`, at
                // which point the async monitor will try to poll it again. This gives it another
                // opportunity to return `std::task::Poll::Ready`, without having to block other
                // tasks in between.
                //
                // Futures are essentially implemented as a state machine which can be stopped and
                // resumed depending on their output. This is abstracted when using `async` blocks,
                // but then we don't get to decide when the future is to be woken again! A key
                // observation is that we don't want progress we made between polling to be lost.
                //
                // Take the example of a tokio mutex: with a bit of digging, we can find out that
                // under the hood `tokio::sync::Mutex::lock` ends up calling a low-level semaphore
                // implementation to handle locking (this is not part of the public-facing API).
                // This calls a _sync_ `acquire` method which returns an `Acquire` struct.
                // `Acquire` itself implements `std::future::Future` and uses the Waiter` struct to
                // keep track of async state. This includes, among other things, keeping track of
                // lock priority to ensure fairness.
                //
                // https://github.com/tokio-rs/tokio/blob/9e7e1ef7ad30ad84f54a047ecd65b78e5973a9c4/tokio/src/sync/batch_semaphore.rs#L71
                //
                // > But what happens if this state is dropped?
                //
                // If the `Acquire` state is dropped, this is like giving up on locking a mutex
                // mid-way and having to start all over again! Any priority which has been
                // established is discarded, resulting in callers racing against each other in no
                // particular order -yikes! Clearly dropping async state when manually implementing
                // a polling state machine is bad, as we loose any progress that state machine had
                // done up to that point :P
                //
                // It's not usual to have to think about this when writing async code, because the
                // Rust compiler will desugar async methods to keep track of that state for you. In
                // our case though we need to take the manual approach. This is what the `State`
                // structure is responsible for: we want to make sure that _if_ a call to
                // `poll_next` exits before it can reach the `std::poll::Ready` state, then any
                // progress made so far is not lost.
                //
                // The advantage to this is that we have much more fine-grained control in telling
                // the async runtime _when_ it needs to stop and resume computation in our stream.
                // This allows use to make progress only when needed, and removes any overhead
                // associated with running separate tasks for producing and receiving data.
                let block = match &mut self.state {
                    State::Polling(fut) => {
                        let (state, block) = match fut.as_mut().poll(cx) {
                            std::task::Poll::Ready((state, block)) => (state, block),
                            std::task::Poll::Pending => {
                                self.waker.replace(cx.waker().clone());
                                return std::task::Poll::Pending;
                            }
                        };

                        self.state = state;
                        block
                    }
                    State::Idle(inner) => {
                        let (eth, stream) = inner.take().unwrap();
                        let mut fut = Box::pin(Self::block_next(eth, stream));

                        match fut.as_mut().poll(cx) {
                            std::task::Poll::Ready((state, block)) => {
                                self.state = state;
                                block
                            }
                            std::task::Poll::Pending => {
                                self.state = State::Polling(fut);
                                self.waker.replace(cx.waker().clone());
                                return std::task::Poll::Pending;
                            }
                        }
                    }
                };

                let block_n = match block {
                    Ok(block_n) => block_n,
                    Err(err) => return std::task::Poll::Ready(Some(Err(err))),
                };

                self.block_head = block_n.saturating_sub(self.maturity_delay);
                self.block_head -= self.block_head % self.interval_attestation.get();
            }

            // Similarly to the catchup limit, we always refer to the actual cache size when
            // calculating the max block to catch up to.
            let height_first = self
                .continuity
                .roots
                .cache
                .first()
                .map(|info| info.block.number())
                .unwrap_or(self.block_stop);
            let height_first = height_first - (height_first % self.interval_attestation.get());

            self.block_n = self
                .block_head
                .min(height_first.saturating_add(common::constants::MAX_CATCHUP.get()));
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
        start_height: common::types::Height,
        start_info: Option<common::types::AttestationInfo>,
        encoding_version: usc_abi_encoding::common::EncodingVersion,
    ) -> Self {
        let max_size: std::num::NonZeroUsize = common::constants::MAX_CATCHUP
            .saturating_add(1) // Inclusive
            .try_into()
            .unwrap();

        Self {
            cache: Vec::with_capacity(max_size.get()),
            prev_digest: start_info.unwrap_or_default().digest,

            roots: CacheRoots::new(max_size, start_height, encoding_version),
        }
    }

    #[tracing::instrument(skip_all, fields(block_stop))]
    async fn update(
        &mut self,
        eth: &mut eth::Client,
        height_stop: common::types::Height,
    ) -> Result<(), Interrupt<Error>> {
        self.roots.update(eth, height_stop).await?;

        let height_first = self
            .cache
            .first()
            .map(|info| info.block_number)
            .unwrap_or(self.roots.boundary);
        let height_next = self
            .cache
            .last()
            // FIXME: overflow breaks other invariants, saturating isn't enough
            .map(|info| info.block_number + 1)
            .unwrap_or(self.roots.boundary);

        if height_stop < height_next {
            return Ok(());
        }

        tracing::debug!(
            height_next,
            height_stop,
            start_height = self.roots.boundary,
            "Computing continuity proof"
        );

        let size_fragment = height_stop as usize - height_next as usize + 1;

        let roots_start = self.roots.cache[0].block.number();
        let index_start = (height_next - roots_start) as usize;
        let index_stop = index_start + size_fragment;

        let size_fragment_roots = self.roots.cache[index_start..].len();

        assert!(
            size_fragment <= size_fragment_roots,
            "{size_fragment} <= {size_fragment_roots}: missing blocks in root cache"
        );

        assert!(
            roots_start <= height_next,
            "{roots_start} <= {height_next}: violated attestation order"
        );

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

        let len_cache = self.cache.len();
        let max_size = self.roots.max_size.get();

        assert!(
            len_cache <= max_size,
            "Invalid continuity cache size: {len_cache} <= {max_size} [({height_first})/{height_next}:{height_stop}]",
        );

        Ok(())
    }
}

impl CacheRoots {
    pub fn new(
        max_size: std::num::NonZeroUsize,
        start_height: common::types::Height,
        encoding_version: usc_abi_encoding::common::EncodingVersion,
    ) -> Self {
        Self {
            cache: Vec::with_capacity(max_size.get()),
            max_size,
            boundary: start_height,
            encoding_version,
        }
    }

    #[tracing::instrument(skip_all)]
    async fn update(
        &mut self,
        eth: &mut eth::Client,
        height_stop: common::types::Height,
    ) -> Result<(), Interrupt<Error>> {
        use futures::FutureExt as _;
        use futures::StreamExt as _;
        use futures::TryStreamExt as _;
        use rayon::iter::IntoParallelIterator as _;
        use rayon::iter::ParallelExtend as _;
        use rayon::iter::ParallelIterator as _;

        let height_first = self
            .cache
            .first()
            .map(|info| info.block.number())
            .unwrap_or(self.boundary);
        let height_next = self
            .cache
            .last()
            // FIXME: overflow breaks other invariants, saturating isn't enough
            .map(|info| info.block.number() + 1)
            .unwrap_or(self.boundary);

        if height_stop < height_next {
            tracing::info!(
                height_next,
                height_stop,
                start_height = self.boundary,
                "🎯 Cache hit"
            );
            return Ok(());
        }

        tracing::info!(
            height_next,
            height_stop,
            start_height = self.boundary,
            "🎯 Cache miss"
        );

        let encoding = self.encoding_version;
        let iter = (height_next..=height_stop).map(|h| {
            eth.get_block(h, encoding)
                .map(|res| res.map_interrupt(Error::Eth))
        });
        let blocks = futures::stream::iter(iter)
            .buffered(common::constants::MAX_CONCURRENT_RPC_CALLS)
            .try_collect::<Vec<_>>()
            .await?;

        self.cache
            .par_extend(blocks.into_par_iter().map(|block| RootInfo {
                root: eth::simple_merkle_tree(&block).root(),
                block,
            }));

        let len_cache = self.cache.len();
        let max_size = self.max_size.get();

        assert!(
            len_cache <= max_size,
            "Invalid roots cache size: {len_cache} <= {max_size} [({height_first})/{height_next}:{height_stop}]",
        );

        assert!(
            !self.cache.is_empty(),
            "Cache cannot be empty after an update [({height_first}){height_next}:{height_stop}]"
        );

        Ok(())
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl StreamAttestation {
    #[tracing::instrument(skip_all, fields(height = info.height, digest = %info.digest))]
    pub fn note_attestation_finalization(&mut self, info: common::types::AttestationInfo) {
        let interval_attestation = self.interval_attestation.get();
        let height = info.height - (info.height % interval_attestation);

        tracing::debug!(
            block_stop = self.block_stop,
            height,
            "Updating attestation stream"
        );

        if self.block_stop < height {
            self.block_stop = height;
        }

        if let Some(waker) = self.waker.take() {
            waker.wake();
        }

        self.continuity.note_attestation_finalization(info)
    }

    pub fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
    ) {
        self.interval_attestation = interval_new;
    }

    #[tracing::instrument(skip_all, fields(height = info.height, digest = %info.digest))]
    pub async fn note_attestation_chain_reversion(
        &mut self,
        info: common::types::AttestationInfo,
    ) -> Result<(), Interrupt<Error>> {
        use futures::StreamExt as _;

        tracing::info!(
            reset_height = info.height,
            "Chain reversion: resetting attestation stream"
        );

        // Re-creating attestation stream state.
        let client_eth = match &self.eth_transport {
            eth::ConnectionTransport::Http(url) => eth::Client::new(url.as_ref(), None)
                .await
                .map_interrupt(|e| Error::ReInitError(e.to_string()))?,
            eth::ConnectionTransport::Ws(ws_connect) => {
                eth::Client::new(ws_connect.url.as_ref(), None)
                    .await
                    .map_interrupt(|e| Error::ReInitError(e.to_string()))?
            }
        };

        let mut stream = client_eth
            .subscribe()
            .await
            .map_interrupt(|e| Error::ReInitError(e.to_string()))?;
        let next = stream
            .next()
            .await
            .ok_interrupt(Error::StreamError)?
            .number
            .saturating_sub(self.maturity_delay);

        self.state = State::Idle(Some((client_eth, stream)));

        // Resetting cache. We can't trust stored blocks or roots after a reversion.
        self.continuity = CacheContinuity::new(info.height, Some(info), self.encoding_version);

        // Resetting key markers
        let interval_attestation = self.interval_attestation.get();

        self.block_head = next - (next % interval_attestation);
        self.block_n = self.block_head.min(
            common::constants::MAX_CATCHUP
                .get()
                .saturating_add(info.height),
        );
        self.block_stop = info.height;
        self.block_start = self.block_n;

        // Waiting for 10 seconds so that other attestors have time to clear their votes
        // before we gossip new ones
        let wait_time_ms = 10000;
        let step = wait_time_ms / 10;

        for i in 1..=10 {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(step)) => {
                    tracing::info!("⏳ Delaying attestation production for synchronization {}/{}ms", step * i, wait_time_ms);
                }
                _ = tokio::signal::ctrl_c() => {
                    return Err(Interrupt::Stop);
                }
            }
        }
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }

        Ok(())
    }
}

impl CacheContinuity {
    fn note_attestation_finalization(&mut self, info: common::types::AttestationInfo) {
        tracing::debug!("Updating continuity cache");

        self.cache.clear();

        if info.height >= self.roots.boundary {
            self.prev_digest = info.digest;
        }

        self.roots.note_attestation_finalization(info)
    }
}

impl CacheRoots {
    fn note_attestation_finalization(&mut self, info: common::types::AttestationInfo) {
        tracing::debug!("Updating roots cache");

        if !self.cache.is_empty() {
            let height_first = self.cache.first().unwrap().block.number();

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

                assert!(
                    removed_last <= info.height,
                    "{removed_last} <= {}",
                    info.height
                );

                self.boundary = info.height.saturating_add(1);
            }
        } else {
            self.boundary = info.height.saturating_add(1);
        }
    }
}
