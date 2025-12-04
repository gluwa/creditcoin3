//! A strongly ordered data structure which efficiently keeps track of pending attestations.
//!
//! # Usage
//!
//! The attestation pool is a structure which stores attestation readiness across threads. It
//! supports first-in-first-out ordering of attestations with eager insertions and lazy retrieval,
//! meaning writes take precedence and reads only take place when there is new data to be
//! examined thanks to an `async` api.
//!
//! A [`sender`] pushes new attestaions into the pool from a [worker thread] whenever a new
//! attestation is made available. This can be a [locally produced attestation], or a remote
//! attestation which has been gossipped via the [p2p layer]. Attestations in the pool are ordered
//! by height and are not checked for correctness. Instead, matching attestations are grouped
//! together and checked for _quorum_. Once quorum has been reached, a waiting [`receiver`] will be
//! woken by the async runtime to be polled.
//!
//! Attestation senders and receivers are `mpsc`, allowing for multiple blocking writers and a
//! single reader. Read and writes are exclusive, so special care must be taken not to starve
//! either end!
//!
//! # Validation
//!
//! Validation takes place on the receiving end of the attestation pool. This is to keep insertions
//! into the pool as fast as possible, but is also an optimization since it means we only validate
//! attestations _after_ they have reached quorum, which reduces the number of time we need to
//! validate the _continuity_ proof of an attestation by a factor of the quorum size.
//!
//! Polling the attestation pool after quorum has been reached does not perform any mutation, and
//! instead returns an [`AttestationPermit`]. This permit _must_ be used by the [validation worker]
//! to mark the attestation as [`valid`] or [`invalid`] and remove it from the pool once it has
//! finished checking it, which is when the mutation occurs. This is done for several reasons:
//!
//! 1. It makes it so polling the attestation pool is cancellation-safe, and can be run inside of a
//!    [`select`] statement.
//! 2. It minimizes the time during which the pool is locked by performing validation outside the
//!    lock.
//!
//! # Batching
//!
//! To optimize for throughput ahead of runtime finality, the attestation pool supports the
//! optimistic batching of attestations with the assumption that attestations which have previously
//! reached quorum locally will be accepted by the runtime. This allows us to batch up to
//! `max_attestations_per_block` in advance to be submitted at once. We do this while waiting on the
//! runtime to validate any previous attestations we sent it, minimizing the amount of time spent
//! idle.
//!
//! In an ideal scenario, the time it takes for the largest possible attestation batch to reach
//! quorum and be validated locally should be equal to the time it takes the runtime to validate the
//! previous batch and finalize it on-chain. That way we can guarantee that no idle time is spent
//! during which the execution chain is not making progress in the finalization of new attestations.
//!
//! In practice, attestors are either able to produce attestations well in advance of the runtime
//! in situations where they are catching up on source chain finality, or else source chain
//! finality is too slow to saturate the bandwidth of the attestor network. This both points to the
//! fact that the runtime can be further optimized to better handle chain catchup and that we can
//! easily support more chains when _not_ in catchup, though catching up to even a single source
//! chain will currently saturate the runtime. Ideally we would want the reverse to be true, so
//! that the runtime can validate attestations much faster than they are produced.
//!
//! # DOS mitigation
//!
//! In order to mitigate spamming risks, the attestation pool has a strictly bounded capacity which
//! is set on initialization. Once this capacity has been reached, the pool will begin to evict
//! attestations at the highest known attestation height, since that is the least likely to reach
//! finality as attestations must be sequential.
//!
//! # Example
//!
//! ```rust
//! # use attestor::worker::validation::pool::attestation_pool;
//! # use attestor::worker::validation::pool::ConfigBuilder;
//! # use attestor::worker::validation::pool::AttestorValidatePermissionless;
//! #
//! # fn attestation(attestor: attestor_primitives::AttestorId) -> attestor::common::types::Attestation {
//! #   attestor::common::types::Attestation {
//! #       attestation_data: attestor_primitives::Attestation {
//! #           header_number: 0,
//! #           prev_digest: Some(sp_core::H256(*b"digest_0________________________")),
//! #           ..Default::default()
//! #       },
//! #       attestor,
//! #       signature: Default::default(),
//! #       signature_bls: attestor_primitives::bls::WrapEncode(
//! #           bls_signatures::PrivateKey::new(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
//! #               .sign(b"0xdeadbeef"),
//! #       ),
//! #       continuity_proof: Default::default(),
//! #       epoch: 0,
//! #   }
//! # }
//! #
//! # fn validate(_quorum: attestor::worker::validation::pool::Quorum) -> bool {
//! #   true
//! # }
//! #
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() {
//! #   use futures::stream::StreamExt as _;
//! #
//! #   let attestation_0 = attestation(attestor_primitives::AttestorId::from_public(*b"attestor_valid_0________________"));
//! #   let attestation_1 = attestation(attestor_primitives::AttestorId::from_public(*b"attestor_valid_1________________"));
//! #   let attestation_2 = attestation(attestor_primitives::AttestorId::from_public(*b"attestor_valid_2________________"));
//! #
//! // Initializes the attestation pool with some configuration
//! let (sx, mut rx) = attestation_pool(
//!     ConfigBuilder::new()
//!         .with_capacity(std::num::NonZeroUsize::new(100).unwrap())
//!         .with_quorum(std::num::NonZeroUsize::new(3).unwrap())
//!         .with_attestors(AttestorValidatePermissionless)
//!         .with_start_height(0u64)
//!         .with_attestation_interval(std::num::NonZeroU64::new(1).unwrap())
//!         .with_max_attestations_per_block(10u32)
//!         .build(),
//! );
//!
//! // Sends 3 attestations at the same height from different attestors
//! sx.send(attestation_0).unwrap();
//! sx.send(attestation_1).unwrap();
//! sx.send(attestation_2).unwrap();
//!
//! // An attestation has reached quorum!
//! let (quorum, permit, digest_local) = rx.next().await.unwrap();
//!
//! // Perform some validation logic and remove the attestation from the pool
//! if validate(quorum) {
//!     rx.mark_valid(permit).unwrap();
//! } else {
//!     rx.mark_invalid(permit).unwrap();
//! }
//! # }
//! ```
//!
//! [`sender`]: AttestationPoolSender
//! [worker thread]: crate::worker
//! [locally produced attestation]: crate::worker::production
//! [p2p layer]: crate::worker::p2p
//! [`receiver`]: AttestationPoolReceiver
//! [validation worker]: crate::worker::validation
//! [`valid`]: AttestationPoolReceiver::mark_valid
//! [`invalid`]: AttestationPoolReceiver::mark_invalid
//! [`select`]: tokio::select

mod error;
mod hash;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, attestor_macro::Builder)]
/// Attestation pool configuration options
pub struct Config {
    /// Maximum number of attestations which can be held in the pool before the pool begins
    /// evicting the highest attestations.
    capacity: std::num::NonZeroUsize,
    /// Attestor validation policy, can be either [`AttestorValidatePermissionless`] or
    /// [`AttestorValidatePermissioned`].
    attestors: Box<dyn AttestorValidate>,
    #[specify_later]
    /// Target [`Quorum`] size. Ie: the number of valid attestors which must submit the same
    /// attestation before it reaches quorum.
    quorum: std::num::NonZeroUsize,
    #[specify_later]
    /// Interval at which attestations are being produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    attestation_interval: std::num::NonZero<common::types::Height>,
    #[specify_later]
    /// Starting height at which attestation are produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    start_height: common::types::Height,
    #[specify_later]
    /// Maximum number of attestations which can be validated in a single block by the runtime.
    /// This is a hard bound and attestation batches greater than this limit will be rejected
    /// outright.
    ///
    /// This value is fetched from on-chain storage.
    max_attestations_per_block: u32,
}

// ------------------------------------ [ Attestation Pool ] ----------------------------------- //

/// Multiple-sender end of the attestation pool. A sender can be cloned to be shared across multiple
/// threads.
pub struct AttestationPoolSender {
    common: std::sync::Arc<AttestationPoolCommon>,
}

/// Single-receiver end of the attestation pool. The attestation pool receiver is exclusive and can
/// only be read by the [validation worker].
///
/// [validation worker]: crate::worker::validation
pub struct AttestationPoolReceiver {
    common: std::sync::Arc<AttestationPoolCommon>,
}

#[derive(Debug, PartialEq, Eq)]
/// Identifying information about an attestation batch, avoids the receiver having to inspect the
/// batch again when this information can be retrieved directly by the attestation pool.
pub struct BatchInfo {
    /// The _previous_ digest of the first attestation in the batch.
    pub digest_first: Option<attestor_primitives::Digest>,
    /// The digest of the last attestation in the batch.
    pub digest_last: attestor_primitives::Digest,
    /// The height of the first attestation in the batch
    pub height_first: common::types::Height,
    /// The height of the last attestation in the batch
    pub height_last: common::types::Height,
}

/// Creates a new attestation pool and returns its [`sender`] and [`receiver`] ends.
///
/// * `capacity`: maximum number of attestations which can be held in the pool before eviction.
/// * `quorum`: settings related to quorum checks.
/// * `attestors`: settings related to attestor checks.
///
/// [`sender`]: AttestationPoolSender
/// [`receiver`]: AttestationPoolReceiver
pub fn attestation_pool(config: Config) -> (AttestationPoolSender, AttestationPoolReceiver) {
    const QUORUM_HIGH: usize = 255;

    if config.quorum.get() > QUORUM_HIGH {
        tracing::warn!(quorum = config.quorum, "⚠️ Abnormally high qorum count");
    }

    tracing::info!("📮 Starting attestor pool");
    tracing::info!(capacity = %config.capacity, "📮  with");
    tracing::info!(height = %config.start_height, "📮  with");
    tracing::info!(interval = %config.attestation_interval, "📮  with");
    tracing::info!(quorum = %config.quorum, "📮  with");
    tracing::info!(attestors = %config.attestors, "📮  with");

    let quorum = QuorumValidate::new(
        config.quorum,
        config.start_height,
        config.attestation_interval,
    );

    let pool = AttestationPool::new(config.capacity, quorum, config.attestors);

    let common_send = std::sync::Arc::new(AttestationPoolCommon::new(
        pool,
        config.start_height,
        config.max_attestations_per_block,
    ));

    let common_recv = std::sync::Arc::clone(&common_send);

    let send = AttestationPoolSender {
        common: common_send,
    };

    let recv = AttestationPoolReceiver {
        common: common_recv,
    };

    (send, recv)
}

/// Shared data between the [`AttestationPoolSender`] and the [`AttestationPoolReceiver`].
struct AttestationPoolCommon {
    /// There are a few very important considerations to be had in our choice of a lock type:
    ///
    /// ## `sync` vs `async`
    ///
    /// All of the operations taking place inside of the attestation pool are strictly
    /// compute-bound: no io operations, all state is stored inside of main memory. Also keep in
    /// mind that we are running each worker in its own single-threaded [`tokio`] runtime, which
    /// means we are blocking the tokio runner anyway regardless of if we use a `sync` or `async`
    /// lock. We are using tokio for asynchronicity, not concurrency, and so this defeats a lot of
    /// the purposes of using [`tokio::sync::Mutex`].
    ///
    /// ## Performance
    ///
    /// Besides there not being any `async` advantage to [`tokio::sync::Mutex`] in our use case, it
    /// must also be noted that tokio's own locking primitives are very slow, due to what seems
    /// mostly like overhead in the Rust `async` state machine. Some benchmarking has been done
    /// (see [mutex-bench]) which shows [`std::sync::Mutex`] and [`parking_lot::Mutex`] performing
    /// respectively around 33x and 70x faster than [`tokio::sync::Mutex`]. Since we don't have to
    /// worry about blocking the tokio runtime worker, this seems like an obvious choice.
    ///
    /// ## Fairness
    ///
    /// Fairness measures a lock's ability to yield access equitably across multiple waiters, so
    /// that one thread cannot continuously starve others by always acquiring the lock faster than
    /// any other.
    ///
    /// Based on the same previous benchmarks, tokio's [`tokio::sync::Mutex`] behaves the best,
    /// however its poor performance make this an undesirable choice. [`std::sync::Mutex`] and
    /// [`parking_lot::Mutex`] compare similarly, with [`std::sync::Mutex`] having better eventual
    /// finality by a slight margin. [`parking_lot::FairMutex`] does not seem to yield much better
    /// fairness than its unfair alternative despite introducing some performance overhead. This
    /// should be unsurprising, as per the docs
    ///
    /// > _"[`parking_lot::Mutex`] uses eventual fairness to ensure that the lock will be fair on
    /// > average without sacrificing throughput. This is done by forcing a fair unlock on average
    /// > every 0.5ms, which will force the lock to go to the next thread waiting for the mutex."_
    ///
    /// Fairness is very important in our use case, as each worker thread will often be racing for
    /// access to the inner attestation pool, either to read, write or remove the latest
    /// attestation quorum. In particular, the `p2p` worker thread tends to pretty aggressively
    /// throttle the lock on the inner attestation pool if it is flooded by a sudden influx of
    /// gossipsub messages. If we are not mindful about lock fairness, this can lead to a situation
    /// where the `p2p` thread always wins the lock acquire race, starving the `validation` and
    /// `production` threads for progress.
    ///
    /// ## Poisoning
    ///
    /// Finally, we need to consider failures in the attestor code. As we eventually aim to
    /// implement a thread healing policy to handle thread failures on-the-fly, we need to be
    /// particularly mindful about poisoning shared resources on a panic. [`parking_lot::Mutex`] is
    /// very interesting in this case as it _cannot be poisoned_. This is in contrast to
    /// [`std::sync::Mutex`] which can be poisoned if a thread panics while holding it.
    ///
    /// ## [`parking_lot`]
    ///
    /// Overall, I've settled for using [`parking_lot::Mutex`] for three main reasons (by order of
    /// importance):
    ///
    /// - Decent fairness, avoiding thread starvation on continuous lock misses.
    /// - Excellent performance.
    /// - Poison-resistance, making it easier to handle cross-thread failure cases in the future.
    ///
    /// This might change in the future if the above assumptions no longer hold, but be sure to
    /// benchmark any change before doing so!
    ///
    /// [mutex-bench]: https://github.com/gluwa/mutex-bench
    pool: parking_lot::Mutex<AttestationPool>,
    count_sender: std::sync::atomic::AtomicUsize,

    // CROSS THREAD ATOMIC DATA STORE
    //
    // Data which needs to be accessed frequently without acquiring a lock onto the inner pool.
    batch_size: std::sync::atomic::AtomicUsize,
    attestation_local: std::sync::atomic::AtomicU64,
    start_height: common::types::Height,
    max_attestations_per_block: u32,
}

impl AttestationPoolCommon {
    pub fn new(
        pool: AttestationPool,
        start_height: common::types::Height,
        max_attestations_per_block: u32,
    ) -> Self {
        Self {
            pool: parking_lot::Mutex::new(pool),
            count_sender: std::sync::atomic::AtomicUsize::new(0),

            batch_size: std::sync::atomic::AtomicUsize::new(0),
            attestation_local: std::sync::atomic::AtomicU64::new(start_height),
            start_height,
            max_attestations_per_block,
        }
    }
}

impl Default for AttestationPoolCommon {
    fn default() -> Self {
        Self {
            pool: parking_lot::Mutex::new(AttestationPool::Closed),
            count_sender: std::sync::atomic::AtomicUsize::new(0),

            batch_size: std::sync::atomic::AtomicUsize::new(0),
            attestation_local: std::sync::atomic::AtomicU64::new(0),
            start_height: 0,
            max_attestations_per_block: 0,
        }
    }
}

/// Attestation pool status. The pool can no longer receiver or retrieve attestations once it is
/// [`Closed`].
///
/// [`Closed`]: AttestationPool::Closed
enum AttestationPool {
    Open(AttestationPoolInner),
    Closed,
}

/// Concrete implementation of the attestation pool, holding all of the implementation logic.
struct AttestationPoolInner {
    // POOL DATA
    heights: std::collections::BTreeMap<common::types::Height, HeightData>,
    capacity: std::num::NonZeroUsize,
    quorum: QuorumValidate,
    attestors: Box<dyn AttestorValidate>,
    wakers: std::collections::VecDeque<std::task::Waker>,

    // CROSS-THREAD DATA STORE
    //
    // Data which needs to be accessed infrequently across threads and must be shared to handle
    // epoch resets. For cross-thread data which needs to be accessed frequently, resolve to
    // storing atomic data in `AttestationPoolCommon` instead. For data which cannot be stored
    // atomically, try and return it as part of other locking operations on the inner pool, such as
    // waiting for quorum.
    batch: Vec<common::types::AttestationSigned>,
    digest_local: Option<cc_client::H256>,
}

impl AttestationPool {
    fn new(
        capacity: std::num::NonZeroUsize,
        quorum: QuorumValidate,
        attestors: Box<dyn AttestorValidate>,
    ) -> Self {
        Self::Open(AttestationPoolInner {
            heights: Default::default(),
            capacity,
            quorum,
            attestors,
            wakers: std::collections::VecDeque::with_capacity(capacity.into()),

            batch: Vec::new(),
            digest_local: None,
        })
    }

    #[cfg(test)]
    fn expect_open(&mut self) -> &mut AttestationPoolInner {
        match self {
            AttestationPool::Open(inner) => inner,
            AttestationPool::Closed => todo!(),
        }
    }
}

impl AttestationPoolInner {
    #[tracing::instrument(skip_all)]
    fn evict_if_necessary(&mut self) {
        // TODO: we might want to replace this by just keeping a manual count of attestations as
        // they are inserted an removed, but for now this seems like a less error-prone way to go
        // about this.
        let count = self
            .heights
            .values()
            .map(|height_data| height_data.signer_count_by_attestation_hash.len())
            .sum::<usize>();

        //
        //              Pool's closed!
        //
        // https://knowyourmeme.com/memes/pools-closed
        if count >= self.capacity.into() {
            tracing::warn!(
                "Attestation pool is full, removing attestations at height {}",
                self.heights
                    .last_key_value()
                    .expect("Pool capacity cannot be zero")
                    .0
            );
            // Very simple eviction policy: if the attestation pool is full, we remove all attestations
            // at the highest know attestation height. We do this to avoid having to deal with the
            // complexities of updating the `HeightData::signers` set. Since we can assume this
            // function will only run in states of high congestion, it is best to make sure the
            // eviction logic introduces as little overhead as possible.
            assert!(
                self.heights.pop_last().is_some(),
                "Pool capacity cannot be zero"
            );
        }
    }
}

// ----------------------------------- [ Attestation Sender ] ---------------------------------- //

impl AttestationPoolSender {
    /// Sends an attestation to the attestation pool. Errors if the pool has already been
    /// [`closed`].
    ///
    /// [`closed`]: Self::close
    pub fn send(&self, attestation: common::types::Attestation) -> Result<(), PoolError> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                let span = tracing::debug_span!("", digest = %attestation.digest());
                let _enter = span.enter();

                tracing::debug!("Validating sender");
                if let Err(err) = inner.attestors.validate(&attestation) {
                    return Err(PoolError::Attestation(err));
                }

                tracing::debug!(
                    target_height = inner.quorum.target_height,
                    "Validating height"
                );

                if attestation.header_number() < inner.quorum.target_height {
                    return Err(PoolError::Attestation(AttestationError::InvalidHeight(
                        attestation.attestor.clone(),
                        attestation.epoch,
                        attestation.header_number(),
                        inner.quorum.target_height,
                    )));
                }

                tracing::debug!("Making sure there is enough space in the pool");

                inner.evict_if_necessary();

                tracing::debug!("Adding attestation to pool");

                let err = match inner.heights.entry(attestation.header_number()) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(HeightData::new(attestation));
                        Ok(())
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        entry.get_mut().receive(attestation)
                    }
                };

                if let Err(err) = err {
                    return Err(PoolError::Attestation(err));
                }

                if let Some(waker) = inner.wakers.pop_back() {
                    tracing::debug!("A receiver was found waiting, waking it up...");
                    waker.wake();
                }

                Ok(())
            }
            AttestationPool::Closed => {
                tracing::error!("Tried to send attestation to pool after it has been closed!");
                Err(PoolError::PoolClosed)
            }
        }
    }

    pub fn attestion_local_get(&self) -> common::types::Height {
        self.common
            .attestation_local
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Closes the attestation pool. Successive calls to [`send`] will error, while polling via the
    /// [`receiver`] end will terminate its [`Stream`].
    ///
    /// [`send`]: Self::send
    /// [`receiver`]: AttestationPoolReceiver
    /// [`Stream`]: futures::Stream
    #[allow(unused)]
    pub fn close(self) {
        *self.common.pool.lock() = AttestationPool::Closed;
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

// Handling in response to execution chain events.
impl AttestationPoolSender {
    /// A new attestation has reached finality on the execution chain.
    ///
    /// Remove all attestations _up to and including_ that attestation height from the inner
    /// attestation pool and the attestation batch.
    pub fn note_attestation_finalization(
        &self,
        latest_attestation_cc3: common::types::Height,
    ) -> Result<(), PoolError> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                // Updating quorum
                inner.quorum.height_update(util::next_multiple_of(
                    inner.quorum.attestation_interval,
                    latest_attestation_cc3,
                ));

                // Updating the inner pool
                inner
                    .heights
                    .retain(|height, _data| *height > latest_attestation_cc3);

                // Updating the attestation batch
                inner
                    .batch
                    .retain(|att| att.header_number() > latest_attestation_cc3);
                self.common
                    .batch_size
                    .store(inner.batch.len(), std::sync::atomic::Ordering::Release);

                Ok(())
            }
            AttestationPool::Closed => Err(PoolError::PoolClosed),
        }
    }

    /// An invalid attestation was submitted to the runtime.
    ///
    /// Remove all attestations _after and including_ that attestation height from the inner
    /// attestation pool and the attestation batch.
    pub fn note_attestation_invalidation(
        &self,
        height: common::types::Height,
    ) -> Result<Option<(attestor_primitives::Digest, common::types::Height)>, PoolError> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                // Updating quorum
                inner.quorum.target_height = height;

                // Updating the inner pool
                inner.heights.retain(|h, _data| *h < height);

                // Updating the attestation batch
                inner.batch.retain(|att| att.header_number() < height);
                self.common
                    .batch_size
                    .store(inner.batch.len(), std::sync::atomic::Ordering::Release);

                // Updates the local view of the attestation chain
                let attestation_local = if let Some((height, data)) = inner.heights.last_key_value()
                {
                    let digest = data
                        .attestations_by_signer_count
                        .last_key_value()
                        .expect("Invariant violated,  height data without any attestations")
                        .1
                        .iter()
                        .next()
                        .expect("Invariant violated,  height data without any attestations")
                        .votes[0]
                        .digest();

                    self.common
                        .attestation_local
                        .store(*height, std::sync::atomic::Ordering::Release);
                    inner.digest_local = Some(cc_client::H256(digest.0));

                    Some((digest, *height))
                } else {
                    self.common.attestation_local.store(
                        self.common.start_height,
                        std::sync::atomic::Ordering::Release,
                    );
                    inner.digest_local = None;

                    None
                };

                // Target height was changed, some attestations might have reached quorum: wake up
                // any waiting receivers if there are any.
                if let Some(waker) = inner.wakers.pop_back() {
                    tracing::debug!("A receiver was found waiting, waking it up...");
                    waker.wake();
                }

                Ok(attestation_local)
            }
            AttestationPool::Closed => Err(PoolError::PoolClosed),
        }
    }

    /// A new attestation interval has been set on-chain.
    //
    // Clear the attestation pool and the attestation batch and update the target height and
    // locally tracked attestation interval.
    pub fn note_attestation_interval_change(
        &self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), PoolError> {
        let target_height_new = if let Some(attestation_latest_cc3) = attestation_latest_cc3 {
            util::next_multiple_of(interval_new, attestation_latest_cc3)
        } else {
            self.common.start_height
        };

        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                // Updating quorum
                inner.quorum.attestation_interval = interval_new;
                inner.quorum.target_height = target_height_new;

                // Updating the inner pool
                inner.heights.clear();

                // Updating the attestation batch
                inner.batch.clear();
                self.common
                    .batch_size
                    .store(0, std::sync::atomic::Ordering::Release);

                Ok(())
            }
            AttestationPool::Closed => Err(PoolError::PoolClosed),
        }
    }

    pub fn note_attestors_elected(
        &self,
        attestors: Vec<cc_client::AccountId32>,
    ) -> Result<(), PoolError> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::warn!("🗂️ Updating the attestor set");

                inner.attestors = Box::new(AttestorValidatePermissioned::new(
                    std::collections::HashSet::from_iter(attestors.into_iter().map(|attestor| {
                        attestor_primitives::AttestorId::new(sp_core::crypto::AccountId32::new(
                            attestor.0,
                        ))
                    })),
                ));

                Ok(())
            }
            AttestationPool::Closed => Err(PoolError::PoolClosed),
        }
    }
}

impl Clone for AttestationPoolSender {
    fn clone(&self) -> Self {
        self.common
            .count_sender
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Self {
            common: std::sync::Arc::clone(&self.common),
        }
    }
}

impl Drop for AttestationPoolSender {
    fn drop(&mut self) {
        let count_sender = self
            .common
            .count_sender
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel)
            .checked_sub(1);
        if let Some(0) = count_sender {
            *self.common.pool.lock() = AttestationPool::Closed;
        }
    }
}

// ---------------------------------- [ Attestation Receiver ] --------------------------------- //

impl AttestationPoolReceiver {
    /// Closes the attestation pool. Successive calls to [`send`] will error, while the
    /// [`receiver`] will terminate its [`Stream`].
    ///
    /// [`send`]: AttestationPoolSender::send
    /// [`receiver`]: AttestationPoolReceiver
    /// [`Stream`]: futures::Stream
    #[allow(unused)]
    pub fn close(self) {
        *self.common.pool.lock() = AttestationPool::Closed;
    }

    /// Marks an attestation as valid, causing it and all other attestations at the same height to
    /// be removed from the attestation pool, as well as the pool's target height to be updated.
    ///
    /// This method also returns a vector of all attestations at that height which could not be
    /// polled from the pool in time. These attestations should be considered invalid.
    ///
    /// Errors if the pool is closed.
    #[tracing::instrument(skip_all, fields(%permit))]
    pub fn mark_valid(
        &self,
        permit: AttestationPermit,
    ) -> Result<Vec<common::types::Attestation>, PoolError> {
        // WARNING: VERY IMPORTANT
        //
        // Acquire the lock to the inner attestation pool BEFORE checking for epoch correctness.
        // Check the below comment for more information.
        let mut lock = self.common.pool.lock();
        let vote = &permit.att.votes[0];

        match &mut *lock {
            AttestationPool::Open(inner) => {
                let height = vote.header_number();
                match inner.heights.remove(&height) {
                    // WARNING: RACE CONDITION!
                    //
                    // It is possible for the production thread to flush the attestation pool AFTER
                    // the validation thread has observed quorum on an attestation and received a
                    // permit to remove it. This old attestation now points to an empty height, so
                    // trying to remove it would violate several invariants! We return an error
                    // instead and leave it to the caller to handle it as necessary.
                    None => Err(PoolError::Attestation(AttestationError::MissingHeight(
                        vote.attestor.clone(),
                        vote.epoch,
                        height,
                    ))),
                    Some(height_data) => {
                        tracing::debug!("Removing valid attestation");
                        let mut invalid =
                            Vec::with_capacity(height_data.attestations_by_signer_count.len());

                        for attestations in height_data.attestations_by_signer_count.into_values() {
                            for attestation in attestations {
                                if attestation != permit.att {
                                    invalid.extend(attestation.votes);
                                }
                            }
                        }

                        inner.quorum.target_height = util::next_multiple_of(
                            inner.quorum.attestation_interval,
                            inner.quorum.target_height,
                        );
                        inner.digest_local = Some(cc_client::H256::from(vote.digest().0));

                        self.common
                            .attestation_local
                            .store(height, std::sync::atomic::Ordering::Release);

                        Ok(invalid)
                    }
                }
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to remove valid attestation from pool after it has been closed"
                );
                Err(PoolError::PoolClosed)
            }
        }
    }

    /// Marks an attestation as valid, causing it and all other attestations at the same height to
    /// be removed from the attestation pool, as well as the pool's target height to be updated.
    ///
    /// This method also returns a vector of all attestations at that height which could not be
    /// polled from the pool in time. These attestations should be considered invalid.
    ///
    /// Contrarily to [`mark_valid`], this method is not used to submit attestations to the
    /// runtime. Instead, attestations removed this way from the pool are still stored temporarily
    /// in the inner pool until [`batch_take`] is called for them to be submitted as one.
    /// Attestations which have been batched can also be invalidated as part of execution chain
    /// events in the [production worker].
    ///
    /// Errors if the pool is closed.
    ///
    /// [`mark_valid`]: Self::mark_valid
    /// [`batch_take`]: Self::batch_take
    /// [production worker]: crate::worker::production
    #[tracing::instrument(skip_all, fields(%permit))]
    pub fn mark_batch(
        &self,
        permit: AttestationPermit,
        signed: common::types::AttestationSigned,
    ) -> Result<Vec<common::types::Attestation>, PoolError> {
        // WARNING: VERY IMPORTANT
        //
        // Acquire the lock to the inner attestation pool BEFORE checking for epoch correctness.
        // Check the below comment for more information.
        let mut lock = self.common.pool.lock();
        let vote = &permit.att.votes[0];

        // WARNING: RACE CONDITION
        //
        // This should be a non-issue as long as the validation worker remains single-threaded with
        // a single source of attestation batching, but this feels safer.
        let batch_size = self.batch_size();
        if batch_size >= self.common.max_attestations_per_block {
            let height = vote.header_number();
            return Err(PoolError::Attestation(AttestationError::MaxBatchSize(
                vote.digest(),
                vote.epoch,
                height,
                batch_size,
            )));
        }

        match &mut *lock {
            AttestationPool::Open(inner) => {
                let height = vote.header_number();
                match inner.heights.remove(&height) {
                    // WARNING: RACE CONDITION!
                    //
                    // It is possible for the production thread to flush the attestation pool AFTER
                    // the validation thread has observed quorum on an attestation and received a
                    // permit to remove it. This old attestation now points to an empty height, so
                    // trying to remove it would violate several invariants! We return an error
                    // instead and leave it to the caller to handle it as necessary.
                    None => Err(PoolError::Attestation(AttestationError::MissingHeight(
                        vote.attestor.clone(),
                        vote.epoch,
                        height,
                    ))),
                    Some(height_data) => {
                        tracing::debug!("Batching attestation");
                        let mut invalid =
                            Vec::with_capacity(height_data.attestations_by_signer_count.len());

                        for attestations in height_data.attestations_by_signer_count.into_values() {
                            for attestation in attestations {
                                if attestation != permit.att {
                                    invalid.extend(attestation.votes);
                                }
                            }
                        }

                        inner.quorum.target_height = util::next_multiple_of(
                            inner.quorum.attestation_interval,
                            inner.quorum.target_height,
                        );
                        inner.digest_local = Some(cc_client::H256::from(vote.digest().0));

                        self.common
                            .batch_size
                            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                        inner.batch.push(signed);

                        Ok(invalid)
                    }
                }
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to remove valid attestation from pool after it has been closed"
                );
                Err(PoolError::PoolClosed)
            }
        }
    }

    /// Marks an attestation as **invalid**, causing it to be removed from the attestation pool. The
    /// pool's target height _is not_ updated.
    ///
    /// Errors if the pool is closed.
    #[tracing::instrument(skip_all, fields(%permit))]
    pub fn mark_invalid(&self, permit: AttestationPermit) -> Result<(), PoolError> {
        // WARNING: VERY IMPORTANT
        //
        // Acquire the lock to the inner attestation pool BEFORE checking for epoch correctness.
        // Check the below comment for more information.
        let mut lock = self.common.pool.lock();
        let vote = &permit.att.votes[0];

        match &mut *lock {
            AttestationPool::Open(inner) => {
                match inner.heights.entry(permit.att.votes[0].header_number()) {
                    // WARNING: RACE CONDITION!
                    //
                    // It is possible for the production thread to flush the attestation pool AFTER
                    // the validation thread has observed quorum on an attestation and received a
                    // permit to remove it. This old attestation now points to an empty height, so
                    // trying to remove it would violate several invariants! We return an error
                    // instead and leave it to the caller to handle it as necessary.
                    std::collections::btree_map::Entry::Vacant(_) => {
                        let height = vote.header_number();
                        Err(PoolError::Attestation(AttestationError::MissingHeight(
                            vote.attestor.clone(),
                            vote.epoch,
                            height,
                        )))
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        tracing::debug!("Removing invalid attestation");
                        entry.get_mut().remove_invalid(permit);
                        Ok(())
                    }
                }
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to remove invalid attestation from pool after it has been closed"
                );
                Err(PoolError::PoolClosed)
            }
        }
    }

    /// Returns the current batch size. New attestation are added to the batch as part of
    /// [`mark_batch`], and are removed when calling [`batch_take`] or by execution chain events in
    /// the [production worker].
    ///
    /// [`mark_batch`]: Self::mark_batch
    /// [`batch_take`]: Self::batch_take
    /// [production worker]: crate::worker::production
    pub fn batch_size(&self) -> u32 {
        self.common
            .batch_size
            .load(std::sync::atomic::Ordering::Acquire) as u32
    }

    /// Retrieves all valid attestations batched with [`mark_batch`] to submit them to the runtime
    /// as one.
    ///
    /// Errors if the pool is closed.
    ///
    /// Returns:
    ///
    /// [`None`] if no batch is available, can happen if the previous batch was invalidated or
    /// there was not enough time to batch attestations between submissions. Returns [`Some`]
    /// otherwise.
    ///
    /// [`mark_batch`]: Self::mark_batch
    pub fn batch_take(&self) -> Result<Option<(BatchInfo, common::types::Batch)>, PoolError> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                if !inner.batch.is_empty() {
                    let attestation_first = inner.batch.first().expect("Checked above");
                    let attestation_last = inner.batch.last().expect("Checked above");

                    let info = BatchInfo {
                        digest_first: attestation_first.prev_digest(),
                        digest_last: attestation_last.digest(),
                        height_first: attestation_first.header_number(),
                        height_last: attestation_last.header_number(),
                    };

                    let batch = inner.batch.drain(..).map(Into::into).collect::<Vec<_>>();

                    self.common
                        .batch_size
                        .store(0, std::sync::atomic::Ordering::Release);
                    self.common
                        .attestation_local
                        .store(info.height_last, std::sync::atomic::Ordering::Release);

                    Ok(Some((info, batch)))
                } else {
                    Ok(None)
                }
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to take attestation batch from pool after it has been closed"
                );
                Err(PoolError::PoolClosed)
            }
        }
    }
}

impl futures::Stream for AttestationPoolReceiver {
    type Item = (Quorum, AttestationPermit, Option<cc_client::H256>);

    /// This future is cancellation-safe, as it does not perform any mutations on the inner pool.
    #[tracing::instrument(skip_all)]
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                // NOTE: BATCHING
                //
                // We stop receivers from polling more attestations if the max number of attestations
                // has already been batched, since quorums after that point can no longer be
                // aggregated or sent to the runtime before the previous batch has been processed.
                let batch_size = self.batch_size() as u64 * inner.quorum.attestation_interval.get();
                if batch_size >= self.common.max_attestations_per_block as u64 {
                    inner.wakers.push_front(cx.waker().clone());
                    std::task::Poll::Pending
                } else {
                    match inner
                        .heights
                        .first_entry()
                        .and_then(|entry| entry.get().next_valid(&inner.quorum))
                    {
                        Some((quorum, permit)) => {
                            tracing::debug!(digest = %quorum.digest(), "Found quorum!");
                            std::task::Poll::Ready(Some((quorum, permit, inner.digest_local)))
                        }
                        None => {
                            tracing::debug!("No quorum found, waiting for new attestations...");
                            inner.wakers.push_front(cx.waker().clone());
                            std::task::Poll::Pending
                        }
                    }
                }
            }
            AttestationPool::Closed => {
                tracing::warn!("Tried to read attestation from pool after it has been closed!");
                std::task::Poll::Ready(None)
            }
        }
    }
}

// --------------------------------- [ Attestation Internals ] --------------------------------- //

#[derive(Clone, Debug)]
/// A wrapper around the [`Attestation`] type used to compare attestations between each other.
///
/// [`Attestation`]: common::types::Attestation
struct AttestationByDigest {
    votes: Vec<common::types::Attestation>,
    signers: std::collections::HashSet<attestor_primitives::AttestorId>,
}

impl AttestationByDigest {
    fn new(attestation: common::types::Attestation) -> Self {
        Self {
            signers: hash_set![attestation.attestor.clone()],
            votes: vec![attestation],
        }
    }
}

impl std::hash::Hash for AttestationByDigest {
    // The hash of an attestations is a commitment to its height and digest. It is used to map
    // together similar attestations from different signers.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // WARNING: INVARIANT
        //
        // We assume in the invariants of the attestation pool that all attestation votes in a
        // single `AttestationForHashing` match the same attestation. This is upheld in
        // `HeightData::receive` during attestation insertion into the pool.
        let vote = &self.votes[0];
        vote.header_number().hash(state);
        vote.digest().hash(state);
    }
}

impl std::cmp::PartialEq for AttestationByDigest {
    fn eq(&self, other: &Self) -> bool {
        let vote_self = &self.votes[0];
        let vote_other = &other.votes[0];
        vote_self.header_number() == vote_other.header_number()
            && vote_self.digest() == vote_other.digest()
    }
}

impl std::cmp::Eq for AttestationByDigest {}

/// An aggregate type of all the votes for a given [`Attestation`]
///
/// [`Attestation`]: common::types::Attestation
#[derive(Debug, PartialEq, Eq)]
pub struct Quorum(Vec<common::types::Attestation>);

impl Quorum {
    pub fn digest(&self) -> attestor_primitives::Digest {
        self.0[0].digest()
    }

    pub fn header_number(&self) -> common::types::Height {
        self.0[0].header_number()
    }

    pub fn chain_key(&self) -> attestor_primitives::ChainKey {
        self.0[0].chain_key()
    }

    pub fn votes(self) -> Vec<common::types::Attestation> {
        self.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
/// An aggregate of all attestations for a given height within the same epoch.
///
/// Attestations stored this way are indexed by signer and ordered by attestations with the _most_
/// signers for efficient quorum retrieval.
struct HeightData {
    attestations_by_signer_count:
        std::collections::BTreeMap<usize, std::collections::HashSet<AttestationByDigest>>,
    signer_count_by_attestation_hash: std::collections::HashMap<
        u64,
        usize,
        std::hash::BuildHasherDefault<hash::IdentityHasherU64>,
    >,
    signers: std::collections::HashSet<attestor_primitives::AttestorId>,
    // FIXME: duplicate data, remove. Can be known when retrieving the height data from the inner
    // pool.
    height: common::types::Height,
}

/// A unique permit which can be used to remove attestation from the attestation pool via
/// [`mark_valid`], [`mark_batch`] and [`mark_invalid`].
///
/// [`mark_valid`]: AttestationPoolReceiver::mark_valid
/// [`mark_batch`]: AttestationPoolReceiver::mark_batch
/// [`mark_invalid`]: AttestationPoolReceiver::mark_invalid
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub struct AttestationPermit {
    att: AttestationByDigest,
    hash: u64,
}

impl HeightData {
    fn new(attestation: common::types::Attestation) -> Self {
        let mut height_data = Self {
            height: attestation.header_number(),
            ..Default::default()
        };
        height_data
            .receive(attestation)
            .expect("Inserting first attestation");
        height_data
    }

    #[tracing::instrument(skip_all)]
    /// Inserts an attestation into the attestation pool, mutation its inner state at that height.
    fn receive(&mut self, attestation: common::types::Attestation) -> Result<(), AttestationError> {
        use std::hash::Hash as _;
        use std::hash::Hasher as _;

        let signer = attestation.attestor.clone();
        let mut attestation = AttestationByDigest::new(attestation);

        // We already store the attestation in `attestations_by_signer_count` so we only use its
        // hash to retrieve the vote count. This avoids duplicating the attestation storage since
        // `std::collections::HashMap` also stores a copy of all keys.
        //
        // Also, since we use `std::hash::DefaultHahser` to hash the attestation, we configure
        // `signer_count_by_attestation_hash` so as to directly use each key as a hash, instead of
        // hashing it again on insertion or retrieval. This avoids duplicated hashing since by
        // default `HashMap` uses `DefaultHasher` anyways via `std::hash::RandomState`.
        //
        // >
        // > See the `hash.rs` file in this module for implementation details.
        // >
        //
        // As far as collisions related to user inputs are concerned, the following in an excerpt
        // from the `HashMap` docs:
        //
        // >
        // > By default, HashMap uses a hashing algorithm selected to provide resistance against
        // > HashDoS attacks. The algorithm is randomly seeded, and a reasonable best-effort is made
        // > to generate this seed from a high quality, secure source of randomness provided by the
        // > host without blocking the program.
        // >
        //
        // See https://en.wikipedia.org/wiki/Collision_attack#Hash_flooding.
        let mut hasher = std::hash::DefaultHasher::new();
        attestation.hash(&mut hasher);
        let hash = hasher.finish();

        match self.signer_count_by_attestation_hash.entry(hash) {
            // CASE 1] This is the first time we receive this attestation
            std::collections::hash_map::Entry::Vacant(entry) => {
                tracing::debug!(height = self.height, "No matching attestation");

                entry.insert(1);

                match self.attestations_by_signer_count.entry(1) {
                    // No other attestation exists at this height with a vote count of 1
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        tracing::debug!("No attestations with vote count 1");
                        entry.insert(hash_set![attestation]);
                    }
                    // Another attestation exists with a vote count of 1
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        tracing::debug!("Found attestations with vote count 1");
                        assert!(
                            entry.get_mut().insert(attestation),
                            "Invariant violated: duplicate attestation in `attestations_by_signer_count`"
                        );
                    }
                }
            }
            // CASE 2] Attestation already exists, adding to previous votes
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                tracing::debug!(height = self.height, "Found matching attestations");

                // Retrieve previous attestation
                let signers = *entry.get();

                let std::collections::btree_map::Entry::Occupied(mut attestations) =
                    self.attestations_by_signer_count.entry(signers)
                else {
                    panic!("Invariant violated: attestation is missing in `attestations_by_signer_count`");
                };

                let mut attestation_prev = attestations.get_mut().take(&attestation).expect(
                    "Invariant violated: attestation is missing in `attestations_by_signer_count`",
                );

                {
                    // We no longer need an `AttestationForHashing` as we will be inserting into an
                    // existing attestation.
                    let attestation = attestation.votes.pop().unwrap();

                    let span = tracing::debug_span!("", digest = %attestation.digest());
                    let _enter = span.enter();

                    tracing::debug!("Retrieved previous attestation");

                    // Check votes for this attestation
                    if self.signers.contains(&attestation.attestor) {
                        if attestation_prev.signers.contains(&attestation.attestor) {
                            // WARNING: remember to restore the original state before exiting
                            attestations.get_mut().insert(attestation_prev);
                            return Err(AttestationError::DoubleVote(
                                signer.clone(),
                                attestation.epoch,
                                attestation.header_number(),
                            ));
                        } else {
                            // WARNING: remember to restore the original state before exiting
                            attestations.get_mut().insert(attestation_prev);
                            return Err(AttestationError::Equivocation(
                                signer.clone(),
                                attestation.epoch,
                                attestation.header_number(),
                            ));
                        }
                    }

                    // Once all votes have been checked, only then do we mutate
                    // the previous attestation
                    attestation_prev
                        .signers
                        .insert(attestation.attestor.clone());
                    attestation_prev.votes.push(attestation);

                    // Clean up attestations
                    if attestations.get().is_empty() {
                        attestations.remove();
                    }

                    tracing::debug!("Finished updating previous attestation");
                }

                // Update the signer count
                assert!(
                    self.attestations_by_signer_count
                        .entry(signers + 1)
                        .or_default()
                        .insert(attestation_prev),
                    "Invariant violated: duplicate attestation in `attestations_by_signer_count`"
                );
                entry.insert(signers + 1);
            }
        }

        // If all went went well we update the list of signers at this height to detect
        // future equivocations
        self.signers.insert(signer);

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(%quorum))]
    /// Retrieves the next attestations at this height which has reached quorum, with a bias
    /// towards the attestations with the most votes.
    fn next_valid(&self, quorum: &QuorumValidate) -> Option<(Quorum, AttestationPermit)> {
        use std::hash::Hash as _;
        use std::hash::Hasher as _;

        self.attestations_by_signer_count
            .last_key_value()
            .and_then(|(_, attestations)| {
                let attestation = attestations
                    .iter()
                    .next()
                    .expect("Invariant violated")
                    .clone();

                tracing::debug!("Checking for quorum");

                quorum.validate(&attestation).then(|| {
                    let mut hasher = std::hash::DefaultHasher::new();
                    attestation.hash(&mut hasher);
                    let hash = hasher.finish();

                    let att = AttestationByDigest::new(attestation.votes[0].clone());
                    let quorum = Quorum(attestation.votes);
                    let permit = AttestationPermit { att, hash };

                    (quorum, permit)
                })
            })
    }

    /// Removes an invalid attestation at this height and updates the indexing accordingly. This
    /// method is called by [`mark_invalid`].
    ///
    /// [`mark_invalid`]: AttestationPoolReceiver::mark_invalid
    fn remove_invalid(&mut self, permit: AttestationPermit) {
        let Some(signers) = self.signer_count_by_attestation_hash.remove(&permit.hash) else {
            unreachable!(
                "Invariant violated: attestation permit referencing unknown entry in `signer_count_by_attestation_hash`"
            );
        };

        let attestations = self.attestations_by_signer_count.entry(signers);
        let std::collections::btree_map::Entry::Occupied(mut attestations) = attestations else {
            unreachable!(
                "Invariant violated: attestation permit referencing unkonwn entry in `attestations_by_signer_count`"
            );
        };

        assert!(
            attestations.get_mut().remove(&permit.att),
            "Invariant violated: attestation permit referencing unknown attestation"
        );

        if attestations.get().is_empty() {
            attestations.remove();
        }

        // NOTE: we do not remove the attestation signers so as to still be able to catch future
        // equivocations.
    }
}

// ------------------------------------ [ Quorum Validation ] ---------------------------------- //

/// Encapsulates quorum information to check if an attestation is ready for polling.
///
/// An attestation is ready for polling when enough different attestors have voted for it and its
/// height is next in line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct QuorumValidate {
    target_quorum: std::num::NonZeroUsize,
    target_height: common::types::Height,
    attestation_interval: std::num::NonZero<common::types::Height>,
}

impl QuorumValidate {
    pub const fn new(
        target_quorum: std::num::NonZeroUsize,
        target_height: common::types::Height,
        attestation_interval: std::num::NonZero<common::types::Height>,
    ) -> Self {
        Self {
            target_quorum,
            target_height,
            attestation_interval,
        }
    }

    pub fn height_update(&mut self, height_new: common::types::Height) {
        if self.target_height < height_new {
            self.target_height = height_new;
        }
    }

    #[tracing::instrument(skip_all, fields(target_height = %self.target_height, target_quorum = %self.target_quorum))]
    fn validate(&self, attestation: &AttestationByDigest) -> bool {
        let vote = &attestation.votes[0];

        tracing::debug!(
            height = vote.header_number(),
            quorum = attestation.signers.len(),
            "Validating attestation"
        );

        vote.header_number() == self.target_height
            && attestation.signers.len() >= self.target_quorum.into()
    }
}

// ----------------------------------- [ Attestor Validation ] --------------------------------- //

/// Common trait used to determine if an attestor can submit attestations.
pub trait AttestorValidate: Send + Sync + std::fmt::Debug + std::fmt::Display + 'static {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), AttestationError>;
}

/// Enforces permissioned attesting.
///
/// Attestors are retrieved on-chain from the currently elected authorities. Any other attestation
/// source is denied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttestorValidatePermissioned {
    attestor_set: std::collections::HashSet<attestor_primitives::AttestorId>,
}

impl AttestorValidatePermissioned {
    pub fn new(attestor_set: std::collections::HashSet<attestor_primitives::AttestorId>) -> Self {
        Self { attestor_set }
    }

    pub fn attestors(&self) -> &std::collections::HashSet<attestor_primitives::AttestorId> {
        &self.attestor_set
    }
}

impl std::fmt::Display for AttestorValidatePermissioned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Permissioned: {:?}", self.attestor_set)
    }
}

impl AttestorValidate for AttestorValidatePermissioned {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), AttestationError> {
        if !self.attestor_set.contains(&attestation.attestor) {
            return Err(AttestationError::Unauthorized(
                attestation.attestor.clone(),
                attestation.epoch,
                attestation.header_number(),
            ));
        }
        Ok(())
    }
}

/// Allows attestations from any arbitrary source.
#[derive(Clone, Debug, Default)]
pub struct AttestorValidatePermissionless;

impl AttestorValidate for AttestorValidatePermissionless {
    fn validate(&self, _attestation: &common::types::Attestation) -> Result<(), AttestationError> {
        Ok(())
    }
}

impl std::fmt::Display for AttestorValidatePermissionless {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Permisionless")
    }
}

/// Denies attestations from any source.
///
/// This is useful as a placeholder while we wait to retrieve the set of active attestors on the
/// next attestation election.
#[derive(Clone, Debug, Default)]
pub struct AttestorValidateDeny;

impl AttestorValidate for AttestorValidateDeny {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), AttestationError> {
        Err(AttestationError::Unauthorized(
            attestation.attestor.clone(),
            attestation.epoch,
            attestation.header_number(),
        ))
    }
}

impl std::fmt::Display for AttestorValidateDeny {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Deny")
    }
}

// ----------------------------------------- [ Display ] --------------------------------------- //

impl std::fmt::Display for QuorumValidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ vote_count: {} }}", self.target_quorum)
    }
}

impl std::fmt::Display for AttestationPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let vote = &self.att.votes[0];
        write!(
            f,
            "{{ hash: {}, epoch: {}, height: {} }}",
            self.hash,
            vote.epoch,
            vote.header_number()
        )
    }
}

// ---------------------------------------- [ Fixtures ] --------------------------------------- //

#[cfg(test)]
pub mod constants {
    pub const ATTESTOR_VALID_0: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_0________________");
    pub const ATTESTOR_VALID_1: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_1________________");
    pub const ATTESTOR_VALID_2: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_2________________");
    pub const ATTESTOR_INVALID: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_invalid________________");

    pub const DIGEST_0: attestor_primitives::Digest =
        sp_core::H256(*b"digest_0________________________");
    pub const DIGEST_1: attestor_primitives::Digest =
        sp_core::H256(*b"digest_1________________________");

    pub const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(10);
}

#[cfg(test)]
pub mod fixtures {
    use super::*;
    use constants::*;

    #[rstest::fixture]
    pub fn attestation(
        #[default([ATTESTOR_VALID_0])] attestors: impl IntoIterator<
            Item = attestor_primitives::AttestorId,
        >,
        #[default(0)] epoch: common::types::Epoch,
        #[default(0)] header_number: common::types::Height,
        #[default(DIGEST_0)] prev_digest: attestor_primitives::Digest,
    ) -> AttestationByDigest {
        attestors.into_iter().fold(
            AttestationByDigest {
                votes: Vec::new(),
                signers: std::collections::HashSet::new(),
            },
            |mut attestation, attestor| {
                attestation.votes.push(common::types::Attestation {
                    attestation_data: attestor_primitives::AttestationData {
                        header_number,
                        prev_digest: Some(prev_digest),
                        ..Default::default()
                    },
                    attestor: attestor.clone(),
                    signature: Default::default(),
                    signature_bls: attestor_primitives::bls::WrapEncode(
                        bls_signatures::PrivateKey::new(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                            .sign(b"0xdeadbeef"),
                    ),
                    continuity_proof: Default::default(),
                    epoch,
                });
                attestation.signers.insert(attestor);
                attestation
            },
        )
    }

    #[rstest::fixture]
    pub fn attestation_signed(
        attestation: AttestationByDigest,
    ) -> common::types::AttestationSigned {
        let att = attestation.votes[0].clone();
        attestor_primitives::SignedAttestation {
            attestation: att.attestation_data,
            signature: [0u8; 96],
            attestors: attestation
                .votes
                .iter()
                .map(|att| att.attestor.clone())
                .collect(),
            continuity_proof: att.continuity_proof,
        }
    }

    #[rstest::fixture]
    pub fn quorum(
        #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
            + Clone,
        #[default(0)] _epoch: common::types::Epoch,
        #[default(0)] _header_number: common::types::Height,
        #[default(DIGEST_0)] _prev_digest: attestor_primitives::Digest,
        #[with(_attestors.clone(), _epoch, _header_number, _prev_digest)]
        attestation: AttestationByDigest,
    ) -> Quorum {
        Quorum(attestation.votes)
    }

    #[rstest::fixture]
    pub fn quorum_validate(#[default(2)] vote_count: usize) -> QuorumValidate {
        QuorumValidate {
            target_quorum: vote_count.try_into().unwrap(),
            target_height: 0,
            attestation_interval: std::num::NonZero::<common::types::Height>::MIN,
        }
    }

    #[rstest::fixture]
    pub fn permissioned(
        #[default([ATTESTOR_VALID_0, ATTESTOR_VALID_1, ATTESTOR_VALID_2])]
        attestor_set: impl IntoIterator<Item = attestor_primitives::AttestorId>,
    ) -> AttestorValidatePermissioned {
        AttestorValidatePermissioned {
            attestor_set: std::collections::HashSet::from_iter(attestor_set),
        }
    }

    #[rstest::fixture]
    pub fn permissionless() -> AttestorValidatePermissionless {
        AttestorValidatePermissionless
    }

    #[rstest::fixture]
    pub fn deny() -> AttestorValidateDeny {
        AttestorValidateDeny
    }

    #[rstest::fixture]
    pub fn config(
        quorum_validate: QuorumValidate,
        #[default(100)] capacity: usize,
        permissioned: AttestorValidatePermissioned,
    ) -> Config {
        ConfigBuilder::new()
            .with_capacity(std::num::NonZeroUsize::new(capacity).unwrap())
            .with_attestors(permissioned)
            .with_quorum(quorum_validate.target_quorum)
            .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
            .with_start_height(common::types::Height::MIN)
            .with_max_attestations_per_block(10u32)
            .build()
    }

    #[rstest::fixture]
    pub fn permit(
        #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
            + Clone,
        #[default(0)] _epoch: common::types::Epoch,
        #[default(0)] _header_number: common::types::Height,
        #[default(DIGEST_0)] _prev_digest: attestor_primitives::Digest,
        #[with(_attestors.clone(), _epoch, _header_number, _prev_digest)]
        attestation: AttestationByDigest,
    ) -> AttestationPermit {
        use std::hash::Hash as _;
        use std::hash::Hasher as _;

        let mut hasher = std::hash::DefaultHasher::new();
        attestation.hash(&mut hasher);
        let hash = hasher.finish();

        AttestationPermit {
            att: attestation,
            hash,
        }
    }
}

// -------------------------------------- [ Sanity Checks ] ------------------------------------ //

#[cfg(test)]
mod test {
    use crate::common::fixtures::*;
    use crate::hash_set;

    use super::constants::*;
    use super::fixtures::*;
    use super::*;

    #[rstest::rstest]
    fn attestation_hash_sanity_1(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationByDigest,
    ) {
        use std::hash::Hash as _;
        use std::hash::Hasher as _;

        let mut hasher = std::hash::DefaultHasher::new();
        attestation_0.hash(&mut hasher);
        let hash_0 = hasher.finish();

        let mut hasher = std::hash::DefaultHasher::new();
        attestation_1.hash(&mut hasher);
        let hash_1 = hasher.finish();

        assert_eq!(hash_0, hash_1);
    }

    #[rstest::rstest]
    fn attestation_hash_sanity_2(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationByDigest,
    ) {
        let mut set = hash_set![attestation_1.clone()];
        assert_eq!(set.take(&attestation_1), Some(attestation_0));
    }

    #[rstest::rstest]
    fn attestation_hash_sanity_3(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 0, 0, DIGEST_0)]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 0, 0, DIGEST_1)]
        attestation_1: AttestationByDigest,
    ) {
        use std::hash::Hash as _;
        use std::hash::Hasher as _;

        let mut hasher = std::hash::DefaultHasher::new();
        attestation_0.hash(&mut hasher);
        let hash_0 = hasher.finish();

        let mut hasher = std::hash::DefaultHasher::new();
        attestation_1.hash(&mut hasher);
        let hash_1 = hasher.finish();

        assert_ne!(hash_0, hash_1);
    }

    #[rstest::rstest]
    fn attestation_hash_sanity_4(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 0, 0, DIGEST_0)]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 0, 0, DIGEST_1)]
        attestation_1: AttestationByDigest,
    ) {
        let mut set = hash_set![attestation_1.clone()];
        assert_ne!(set.take(&attestation_1), Some(attestation_0));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_basic(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationByDigest,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        quorum: Quorum,
        #[from(permit)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        permit: AttestationPermit,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.votes[0].clone()).is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).is_ok());

        let actual = rx.next().await;
        let expected = Some((quorum, permit, None));

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_valid(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 0, 0, DIGEST_0)]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 0, 0, DIGEST_0)]
        attestation_1: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_2], 0, 0, DIGEST_1)]
        attestation_2: AttestationByDigest,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 0, 0, DIGEST_0)]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.votes[0].clone()).is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).is_ok());
        assert!(sx.send(attestation_2.votes[0].clone()).is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);

        let invalid = rx.mark_valid(permit).unwrap();
        assert_eq!(invalid, attestation_2.votes);

        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert!(!inner.heights.contains_key(&0));
        assert_eq!(
            inner.digest_local,
            Some(cc_client::H256(attestation_1.votes[0].digest().0))
        );
        assert_eq!(sx.attestion_local_get(), 0);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_invalid(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationByDigest,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.votes[0].clone()).is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);
        assert!(rx.mark_invalid(permit).is_ok());

        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();
        let height_data = inner.heights.get(&0).unwrap();

        assert_eq!(inner.heights.len(), 1);
        assert_eq!(
            height_data.signers,
            hash_set![ATTESTOR_VALID_0, ATTESTOR_VALID_1]
        );
        assert!(height_data.signer_count_by_attestation_hash.is_empty());
        assert!(height_data.attestations_by_signer_count.is_empty());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_batch(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationByDigest,
        #[from(attestation_signed)] attestation_signed: common::types::AttestationSigned,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert_matches::assert_matches!(rx.batch_take(), Ok(None));

        assert!(sx.send(attestation_0.votes[0].clone()).is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);
        assert!(rx.mark_batch(permit, attestation_signed.clone()).is_ok());

        let batch_info_expected = BatchInfo {
            digest_first: attestation_signed.prev_digest(),
            digest_last: attestation_signed.digest(),
            height_first: 0,
            height_last: 0,
        };

        // Such types, much wow... -fuck subxt and the incompatible dependencies which make using
        // our own types an even more royal pain $$%%^#$#
        let batch_expected: Vec<
            cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
                cc_client::H256,
                cc_client::AccountId32,
            >,
        > = vec![attestation_signed.clone().into()];

        assert_matches::assert_matches!(rx.batch_take(), Ok(Some((batch_info, batch))) => {
            assert_eq!(batch_info, batch_info_expected);
            batch.into_iter().zip(batch_expected).for_each(|(att, att_expected)| {
                // Other types in this don't implement PartialEq and Eq...
                assert_eq!(att.attestors, att_expected.attestors);
            });
        });

        assert_eq!(sx.attestion_local_get(), 0);
        assert_eq!(
            sx.common.pool.lock().expect_open().digest_local,
            Some(cc_client::H256(attestation_signed.digest().0))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_evict(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_1: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_2: AttestationByDigest,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_1])]
        quorum: Quorum,
        #[from(permit)]
        #[with([ATTESTOR_VALID_1])]
        permit: AttestationPermit,
        #[from(quorum_validate)]
        #[with(1)]
        _quorum_validate: QuorumValidate,
        #[from(config)]
        #[with(_quorum_validate.clone(), 1)]
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_1.votes[0].clone()).is_ok());
        assert!(sx.send(attestation_2.votes[0].clone()).is_ok());

        let actual = rx.next().await;
        let expected = Some((quorum, permit, None));

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_err_invalid_attestor(
        #[with([ATTESTOR_INVALID])] attestation: AttestationByDigest,
        config: Config,
    ) {
        let (sx, _rx) = attestation_pool(config);

        assert_matches::assert_matches!(
            sx.send(attestation.votes[0].clone()),
            Err(PoolError::Attestation(AttestationError::Unauthorized(
                ATTESTOR_INVALID,
                0,
                0
            )))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_async_wake_receiver(
        _logs: (),
        #[with([ATTESTOR_VALID_0])] attestation: AttestationByDigest,
        #[with([ATTESTOR_VALID_0])] permit: AttestationPermit,
        #[with([ATTESTOR_VALID_0])] quorum: Quorum,
        #[from(quorum_validate)]
        #[with(1)]
        _quorum_validate: QuorumValidate,
        #[with(_quorum_validate.clone())] config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);
        let mut fut = tokio_test::task::spawn(rx.next());

        tokio_test::assert_pending!(fut.poll());
        assert!(sx.send(attestation.votes[0].clone()).is_ok());
        tokio_test::assert_ready_eq!(fut.poll(), Some((quorum, permit, None)));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_async_close(_logs: (), config: Config) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);
        let mut fut = tokio_test::task::spawn(rx.next());

        tokio_test::assert_pending!(fut.poll());
        sx.close();
        tokio_test::assert_ready_eq!(fut.poll(), None);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_close_sender(
        _logs: (),
        #[with([ATTESTOR_VALID_1])] attestation: AttestationByDigest,
        config: Config,
    ) {
        let (sx, rx) = attestation_pool(config);
        rx.close();
        assert_matches::assert_matches!(
            sx.send(attestation.votes[0].clone()),
            Err(PoolError::PoolClosed)
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_close_receiver(
        _logs: (),
        #[with([ATTESTOR_VALID_1])] attestation: AttestationByDigest,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);
        assert!(sx.send(attestation.votes[0].clone()).is_ok());

        sx.close();
        assert!(rx.next().await.is_none());
    }

    #[rstest::rstest]
    fn quorum_parameters_validate(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_1: AttestationByDigest,
        quorum_validate: QuorumValidate,
    ) {
        assert!(quorum_validate.validate(&attestation_0));
        assert!(!quorum_validate.validate(&attestation_1));
    }

    #[rstest::rstest]
    fn validator_parameters_validate_permissioned(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationByDigest,
        permissioned: AttestorValidatePermissioned,
    ) {
        assert!(permissioned.validate(&attestation_0.votes[0]).is_ok());
        assert_matches::assert_matches!(
            permissioned.validate(&attestation_2.votes[0]),
            Err(AttestationError::Unauthorized(ATTESTOR_INVALID, 0, 0))
        );
    }

    #[rstest::rstest]
    fn validator_parameters_validate_permissionless(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationByDigest,
        permissionless: AttestorValidatePermissionless,
    ) {
        assert!(permissionless.validate(&attestation_0.votes[0]).is_ok());
        assert!(permissionless.validate(&attestation_2.votes[0]).is_ok());
    }

    #[rstest::rstest]
    fn validator_parameters_validate_deny(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationByDigest,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationByDigest,
        deny: AttestorValidateDeny,
    ) {
        assert_matches::assert_matches!(
            deny.validate(&attestation_0.votes[0]),
            Err(AttestationError::Unauthorized(ATTESTOR_VALID_0, 0, 0))
        );
        assert_matches::assert_matches!(
            deny.validate(&attestation_2.votes[0]),
            Err(AttestationError::Unauthorized(ATTESTOR_INVALID, 0, 0))
        );
    }
}
