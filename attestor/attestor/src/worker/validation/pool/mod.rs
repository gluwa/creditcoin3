//! A strongly ordered data structure which efficiently keeps track of pending attestations.
//!
//! # Usage
//!
//! The attestation pool implements an ordered queue structure which stores attestation readiness
//! across threads. It supports first-in-first-out ordering of attestations with eager insertions
//! and lazy retrieval, meaning writes take precedence and reads only take place when there is new
//! data to be examined thanks to an `async` api.
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
//! instead returns a [`Permit`]. This permit _must_ be used by the [validation worker] to mark the
//! attestation as [`valid`] or [`invalid`] and remove it from the pool once it has finished
//! checking it, which is when the mutation occurs. This is done for several reasons:
//!
//! 1. It makes it so polling the attestation pool is cancellation-safe, and can be run inside of a
//!    [`select`] statement.
//! 2. It minimizes the time during which the pool is locked by performing validation outside the
//!    lock.
//!
//! # Optimistic production
//!
//! To optimize for throughput ahead of runtime finality, the attestation pool supports the
//! optimistic production of attestations with the assumption that attestations which have
//! previously reached quorum locally will be accepted by the runtime. This allows us to keep
//! producing and validating new attestation in advance of execution chain finality. We do this
//! while waiting on the runtime to validate any previous attestations we sent it, decoupling
//! the production of new attestations from their finalization on the execution chain.
//!
//! # Example
//!
//! ```rust
//! # use attestor::prelude::*;
//! # use attestor::worker::validation::pool::attestation_pool;
//! # use attestor::worker::validation::pool::ConfigBuilder;
//! #
//! # fn attestation(attestor: attestor_primitives::AttestorId) -> common::types::Attestation {
//! #   common::types::Attestation {
//! #       attestation_data: attestor_primitives::AttestationData {
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
//! #   let config = attestor::worker::api::metrics::ConfigBuilder::new()
//! #       .with_name("test")
//! #       .with_address(cc_client::AccountId32([0; 32]))
//! #       .with_peer_id(libp2p::PeerId::random())
//! #       .with_chain_key(2u64)
//! #       .with_start_height(common::types::Height::MIN)
//! #       .with_genesis(common::types::Height::MIN)
//! #       .with_attestation_latest_eth(common::types::Height::MIN)
//! #       .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
//! #       .build();
//! #   let metrics = std::sync::Arc::new(attestor::worker::api::metrics::Metrics::new(config));
//! #   let attestors = vec![
//! #       cc_client::AccountId32(*b"attestor_valid_0________________"),
//! #       cc_client::AccountId32(*b"attestor_valid_1________________"),
//! #       cc_client::AccountId32(*b"attestor_valid_2________________"),
//! #   ];
//! #
//! // Initializes the attestation pool with some configuration
//! let (sx, mut rx) = attestation_pool(
//!     ConfigBuilder::new()
//!         .with_max_size(std::num::NonZeroUsize::new(100).unwrap())
//!         .with_attestors(attestors)
//!         .with_quorum(std::num::NonZeroUsize::new(3).unwrap())
//!         .with_attestation_interval(std::num::NonZeroU64::new(1).unwrap())
//!         .with_start_height(0u64)
//!         .with_max_catchup(std::num::NonZeroU64::new(500).unwrap())
//!         .with_metrics(metrics)
//!         .build(),
//! );
//!
//! // Sends 3 attestations at the same height from different attestors
//! sx.send(attestation_0).unwrap().unwrap();
//! sx.send(attestation_1).unwrap().unwrap();
//! sx.send(attestation_2).unwrap().unwrap();
//!
//! // An attestation has reached quorum!
//! let (quorum, permit, digest_local) = rx.next().await.unwrap();
//!
//! // Perform some validation logic and remove the attestation from the pool
//! if validate(quorum) {
//!     rx.mark_valid(permit);
//! } else {
//!     rx.mark_invalid(permit);
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

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(Debug, builder::Builder)]
/// Attestation pool configuration options
pub struct Config {
    /// Active attestors
    attestors: Vec<cc_client::AccountId32>,

    /// Target [`Quorum`] size. Ie: the number of valid attestors which must submit the same
    /// attestation before it reaches quorum.
    quorum: std::num::NonZeroUsize,

    /// Interval at which attestations are being produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    attestation_interval: std::num::NonZero<common::types::Height>,

    /// Starting height at which attestation are produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    start_height: common::types::Height,

    /// Maximum number of attestation intervals an attestor can catch up from the latest finalized
    /// attestation. Votes beyond this window are rejected to prevent pool-filling DoS attacks.
    max_catchup: std::num::NonZero<common::types::Height>,

    /// Latest execution chain digest, used to validate the tail prev digest of new attestations.
    start_attestation: Option<stream::util::AttestationInfo>,

    metrics: common::types::Metrics,
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

/// Creates a new attestation pool and returns its [`sender`] and [`receiver`] ends.
///
/// [`sender`]: AttestationPoolSender
/// [`receiver`]: AttestationPoolReceiver
pub fn attestation_pool(config: Config) -> (AttestationPoolSender, AttestationPoolReceiver) {
    const QUORUM_HIGH: usize = 255;

    if config.quorum.get() > QUORUM_HIGH {
        tracing::warn!(quorum = config.quorum, "⚠️ Abnormally high qorum count");
    }

    tracing::info!("📮 Starting attestor pool");
    tracing::info!(height = %config.start_height, "📮  with");
    tracing::info!(interval = %config.attestation_interval, "📮  with");
    tracing::info!(quorum = %config.quorum, "📮  with");

    let attestors = ValidateAttestor::new(config.attestors);
    let quorum = ValidateQuorum::new(
        config.quorum,
        config.attestation_interval,
        config.start_height,
        config.max_catchup,
    );

    let last_finalized_height = config.start_attestation.as_ref().map(|info| info.height);

    let pool = AttestationPool::new(
        quorum,
        attestors,
        config.metrics,
        config.start_attestation.map(|info| info.digest),
        last_finalized_height,
    );

    let common_send = std::sync::Arc::new(AttestationPoolCommon::new(pool));
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
}

impl AttestationPoolCommon {
    pub fn new(pool: AttestationPool) -> Self {
        Self {
            pool: parking_lot::Mutex::new(pool),
            count_sender: std::sync::atomic::AtomicUsize::new(1),
        }
    }
}

impl Default for AttestationPoolCommon {
    fn default() -> Self {
        Self {
            pool: parking_lot::Mutex::new(AttestationPool::Closed),
            count_sender: std::sync::atomic::AtomicUsize::new(1),
        }
    }
}

/// Attestation pool status. The pool can no longer receiver or retrieve attestations once it has
/// been closed.
#[allow(clippy::large_enum_variant)]
enum AttestationPool {
    Open(AttestationPoolInner),
    Closed,
}

/// Concrete implementation of the attestation pool, holding all of the implementation logic.
struct AttestationPoolInner {
    forks: AttestationPoolForks,
    valid: AttestationPoolValid,
    digest_local: Option<cc_client::H256>,

    validate_attestor: ValidateAttestor,

    metrics: common::types::Metrics,
    attestation_delay: AttestationPoolDelays,

    wakers: std::collections::VecDeque<std::task::Waker>,
}

impl AttestationPool {
    fn new(
        validate_quorum: ValidateQuorum,
        validate_attestor: ValidateAttestor,
        metrics: common::types::Metrics,
        prev_digest: Option<attestor_primitives::Digest>,
        last_finalized_height: Option<common::types::Height>,
    ) -> Self {
        Self::Open(AttestationPoolInner {
            forks: AttestationPoolForks::new(prev_digest, last_finalized_height, validate_quorum),
            valid: AttestationPoolValid::new(),
            digest_local: None,

            validate_attestor,

            attestation_delay: AttestationPoolDelays::new(metrics.clone()),
            metrics,

            wakers: std::collections::VecDeque::new(),
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
    fn push(&mut self, attestation: common::types::Attestation) -> Result<(), Error> {
        let height = attestation.header_number();

        tracing::debug!("Validating sender");
        self.validate_attestor.validate(&attestation)?;

        tracing::debug!("Adding attestation to pool");
        self.forks.push(attestation)?;

        tracing::trace!("Updating metrics");
        self.attestation_delay.push(height);

        if let Some(waker) = self.wakers.pop_back() {
            tracing::debug!("A receiver was found waiting, waking it up...");
            waker.wake();
        }

        Ok(())
    }

    fn peek(&mut self) -> Option<(Quorum, Permit)> {
        self.forks.peek().map(|fork| {
            let quorum = Quorum(fork.votes.clone());
            let height = fork.attestation.header_number();
            let digest = fork.attestation.digest();
            let header_hash = fork.attestation.attestation_data.header_hash;

            let height_prev =
                height.saturating_sub(fork.attestation.continuity_proof.len() as u64 + 1);
            let digest_continuity = fork
                .attestation
                .continuity_proof
                .compute_continuity_digest(height_prev);

            let permit = Permit(CompoundInfo {
                height,
                digest,
                digest_continuity,
                header_hash,
            });

            // Only update metrics the first time quorum is reached at that height
            if let Some(elapsed) = self.attestation_delay.pop(height) {
                tracing::debug!(
                    ?digest,
                    height,
                    elapsed_ms = elapsed.as_millis(),
                    "⏱️ Time from first vote to quorum"
                );
                self.metrics.update_attestation_delay_quorum(elapsed);
            }

            (quorum, permit)
        })
    }

    fn mark_valid(&mut self, Permit(info): Permit) {
        self.forks.split_off(info.height);
        self.forks.forks_best = self.forks.find_best();
        self.digest_local = Some(cc_client::H256::from(info.digest.0));
    }

    fn mark_invalid(&mut self, Permit(info): Permit) {
        self.forks.pop(info.into());
    }

    fn mark_for_later(
        &mut self,
        permit: Permit,
        signed: common::types::AttestationSigned,
        votes: Vec<common::types::Attestation>,
    ) {
        self.valid.push(signed, votes);
        self.mark_valid(permit);
    }
}

/// Orders attestations by height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyHeight {
    height: common::types::Height,
    size: usize,
    digest: CompoundDigest,
}

/// Orders attestations by quorum size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeySize {
    size: usize,
    height: common::types::Height,
    digest: CompoundDigest,
}

/// Orders attestor votes by height.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyVote {
    height: common::types::Height,
    attestor: attestor_primitives::AttestorId,
}

/// Orders votes by their digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyDigest {
    height: common::types::Height,
    digest: CompoundDigest,
}

/// Orders pending votes by their digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyHeightPending {
    height: common::types::Height,
    digest: CompoundDigest,
    prev_digest_tail: PrevDigestTail,
}

/// Orders pending votes by their prev tail digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyTailPending {
    prev_digest_tail: PrevDigestTail,
    height: common::types::Height,
    digest: CompoundDigest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyDigestPending {
    height: common::types::Height,
    digest: CompoundDigest,
    prev_digest_tail: PrevDigestTail,
}

/// Attestation [digest computation] does not account for all fields in the [`AttestationData`].
/// Namely, the attestation `header_hash` is absent from digest computation yet is still used for
/// [attestation data serialization], **which is what attestors sign on**. This means the
/// attestation digest alone is not a guarantee of uniqueness, and must be paired with the header
/// hash to avoid collisions.
///
/// [digest computation]: attestor_primitives::Attestation::digest
/// [`AttestationData`]:  attestor_primitives::AttestationData
/// [attestation data serialization]: attestor_primitives::AttestationData::serialize
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CompoundDigest {
    digest: attestor_primitives::Digest,
    digest_continuity: attestor_primitives::Digest,
    header_hash: attestor_primitives::Digest,
}

impl CompoundDigest {
    fn min() -> Self {
        Self {
            digest: attestor_primitives::Digest::zero(),
            digest_continuity: attestor_primitives::Digest::zero(),
            header_hash: attestor_primitives::Digest::zero(),
        }
    }

    fn max() -> Self {
        Self {
            digest: attestor_primitives::Digest::from([u8::MAX; 32]),
            digest_continuity: attestor_primitives::Digest::from([u8::MAX; 32]),
            header_hash: attestor_primitives::Digest::from([u8::MAX; 32]),
        }
    }
}

/// Identifying wrapper around [`AttestationInfo`], similar to [`CompoundDigest`].
///
/// [`AttestationInfo`]: stream::util::AttestationInfo
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct CompoundInfo {
    height: common::types::Height,
    digest: attestor_primitives::Digest,
    digest_continuity: attestor_primitives::Digest,
    header_hash: attestor_primitives::Digest,
}

impl From<CompoundInfo> for CompoundDigest {
    fn from(info: CompoundInfo) -> Self {
        Self {
            digest: info.digest,
            digest_continuity: info.digest_continuity,
            header_hash: info.header_hash,
        }
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PrevDigestTail(attestor_primitives::Digest);

/// Holds and manages all attestation forks behind the execution chain finality. Keeps track of
/// contentious forks, past equivocations and known invalid votes. Attestation [`Quorum`]s which can
/// be validated ahead of finality are stored separately in an unbounded collection.
///
/// ## Indexing
///
/// We use compound keys for fast, cache-local indexing.
///
/// > Order matters! [`KeyHeight`] and [`KeySize`] have the same fields but a different ordering:
/// > `KeyHeight` uses the attestation height as its primary key, while `KeySize` uses the quorum
/// > size instead.
///
/// Compound keys are useful when we want to iterate over a large range of related values in an
/// ordered manner, or in case we only need to check for the existence of a given value. They
/// cannot be used to retrieve a value which was not already a part of the key however, and so
/// should not be used to express mappings. Most importantly though, they are very good at
/// condensing multiple orderings into a single tree data structure which improves cache locality
/// and indexing speed.
pub struct AttestationPoolForks {
    forks_by_digest: std::collections::BTreeMap<CompoundDigest, AttestationVote>,
    forks_by_height: std::collections::BTreeSet<KeyHeight>,
    forks_by_size: std::collections::BTreeSet<KeySize>,
    forks_best: Option<AttestationVote>,

    pending_by_digest: std::collections::BTreeMap<KeyDigestPending, AttestationVote>,
    pending_by_prev_digest_tail: std::collections::BTreeSet<KeyTailPending>,
    pending_by_height: std::collections::BTreeSet<KeyHeightPending>,

    votes: std::collections::BTreeMap<KeyVote, CompoundDigest>,
    votes_invalid: std::collections::BTreeSet<KeyDigest>,

    quorums_by_height: std::collections::BTreeSet<KeyHeight>,

    last_finalized_digest: Option<attestor_primitives::Digest>,
    last_finalized_height: Option<common::types::Height>,
    validate_quorum: ValidateQuorum,
}

impl AttestationPoolForks {
    fn new(
        last_finalized_digest: Option<attestor_primitives::Digest>,
        last_finalized_height: Option<common::types::Height>,
        validate_quorum: ValidateQuorum,
    ) -> Self {
        Self {
            forks_by_digest: Default::default(),
            forks_by_height: Default::default(),
            forks_by_size: Default::default(),
            forks_best: Default::default(),

            pending_by_digest: Default::default(),
            pending_by_prev_digest_tail: Default::default(),
            pending_by_height: Default::default(),

            votes: Default::default(),
            votes_invalid: Default::default(),

            quorums_by_height: Default::default(),

            last_finalized_digest,
            last_finalized_height,
            validate_quorum,
        }
    }

    fn push(&mut self, attestation: common::types::Attestation) -> Result<(), Error> {
        let height = attestation.header_number();
        let digest = attestation.digest();
        let attestor = attestation.attestor_id();
        let header_hash = attestation.attestation_data.header_hash;

        let height_prev = height
            .checked_sub(attestation.continuity_proof.len() as u64 + 1)
            .ok_or(Error::InvalidContinuityProof(attestor.clone(), height))?;
        let digest_continuity = attestation
            .continuity_proof
            .compute_continuity_digest(height_prev);

        tracing::debug!("Checking for known invalids");

        let digest = CompoundDigest {
            digest,
            digest_continuity,
            header_hash,
        };

        let key_digest = KeyDigest { height, digest };
        if self.votes_invalid.contains(&key_digest) {
            return Err(Error::InvalidDigest(attestor, height, digest.digest));
        }

        tracing::debug!("Validating attestation height");

        if !self
            .validate_quorum
            .validate_height(height, self.last_finalized_height)
        {
            return Err(Error::InvalidHeight(
                attestor,
                height,
                self.last_finalized_height
                    .unwrap_or(self.validate_quorum.start_height),
            ));
        }

        tracing::debug!("Validating tail prev digest");

        let prev_digest_tail = attestation.continuity_proof.tail_prev_digest();

        let key_vote = KeyVote {
            height,
            attestor: attestor.clone(),
        };

        tracing::debug!("Checking for equivocations");

        match self.votes.entry(key_vote) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(digest);
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                let digest_vote = entry.get();
                if &digest == digest_vote {
                    return Ok(());
                } else {
                    return Err(Error::Equivocation(attestor, height));
                }
            }
        }

        if prev_digest_tail != self.last_finalized_digest {
            tracing::warn!(
                last_finalized_digest = ?self.last_finalized_digest,
                 prev_digest_tail = ?prev_digest_tail,
                "🏎️ Received pending attestation"
            );

            if let Some(prev_digest_tail) = prev_digest_tail.map(PrevDigestTail) {
                let key_digest_pending = KeyDigestPending {
                    height,
                    digest,
                    prev_digest_tail,
                };
                let key_tail_pending = KeyTailPending {
                    prev_digest_tail,
                    height,
                    digest,
                };
                let key_height_pending = KeyHeightPending {
                    height,
                    digest,
                    prev_digest_tail,
                };

                let vote_new = AttestationVote::new(attestation);
                match self.pending_by_digest.entry(key_digest_pending) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(vote_new);

                        assert!(
                            self.pending_by_prev_digest_tail.insert(key_tail_pending),
                            "Duplicate mapping in pending_by_prev_digest_tail: {key_tail_pending:#?}"
                        );
                        assert!(
                            self.pending_by_height.insert(key_height_pending),
                            "Duplicate mapping in pending_by_height: {key_height_pending:#?}"
                        );
                    }
                    std::collections::btree_map::Entry::Occupied(mut entry) => {
                        let vote_prev = entry.get_mut();

                        assert!(!vote_prev.signers.contains(&vote_new.attestation.attestor));

                        vote_prev.votes.extend(vote_new.votes);
                        vote_prev.signers.extend(vote_new.signers);

                        assert!(
                            self.pending_by_prev_digest_tail.contains(&key_tail_pending),
                            "Missing mapping in pending_by_prev_digest_tail: {key_tail_pending:#?}"
                        );
                        assert!(
                            self.pending_by_height.contains(&key_height_pending),
                            "Missing mapping in pending_by_height: {key_height_pending:#?}"
                        );
                    }
                }
            }
        } else {
            tracing::debug!("Inserting attestation");

            let mut vote_new = AttestationVote::new(attestation);
            if let Some(vote_prev) = self.forks_by_digest.remove(&digest) {
                let size = vote_prev.signers.len();
                let key_height_prev = KeyHeight {
                    height,
                    size,
                    digest,
                };
                let key_size_prev = KeySize {
                    size,
                    height,
                    digest,
                };

                assert!(
                    self.forks_by_height.remove(&key_height_prev),
                    "Missing mapping in forks_by_height: {key_height_prev:#?}"
                );
                assert!(
                    self.forks_by_size.remove(&key_size_prev),
                    "Missing mapping in forks_by_size: {key_size_prev:#?}"
                );

                if self.validate_quorum.validate(&vote_prev) {
                    assert!(
                        self.quorums_by_height.remove(&key_height_prev),
                        "Missing mapping in quorums_by_height: {key_height_prev:#?}"
                    );
                }

                vote_new.update(vote_prev);
            }

            let size = vote_new.signers.len();
            let key_height_new = KeyHeight {
                height,
                size,
                digest,
            };
            let key_size_new = KeySize {
                size,
                height,
                digest,
            };

            assert!(
                self.forks_by_height.insert(key_height_new),
                "Duplicate mapping in forks_by_height: {key_height_new:#?}"
            );
            assert!(
                self.forks_by_size.insert(key_size_new),
                "Duplicate mapping in forks_by_size: {key_size_new:#?}"
            );

            if self.validate_quorum.validate(&vote_new) {
                assert!(
                    self.quorums_by_height.insert(key_height_new),
                    "Duplicate mapping in quorums_by_height: {key_height_new:#?}"
                );
            }

            if self.forks_best.as_ref().is_none_or(|best| {
                if self.validate_quorum.validate(best) {
                    self.validate_quorum.validate(&vote_new)
                        && vote_new.attestation.header_number() > best.attestation.header_number()
                } else {
                    vote_new.signers.len() > best.signers.len()
                }
            }) {
                self.forks_best = Some(vote_new.clone());
            }

            assert!(
                self.forks_by_digest.insert(digest, vote_new).is_none(),
                "Duplicate mapping in forks_by_digest: {digest:?}"
            );
        }

        Ok(())
    }

    fn peek(&self) -> Option<AttestationVote> {
        self.forks_best
            .as_ref()
            .and_then(|best| self.validate_quorum.validate(best).then(|| best.clone()))
    }

    fn pop(&mut self, digest: CompoundDigest) {
        let Some(vote) = self.forks_by_digest.remove(&digest) else {
            // NOTE: quorum was picked up right before note_attestation_finalization could run, and
            // has since already been removed from the pool.
            return;
        };

        let height = vote.attestation.header_number();
        let size = vote.signers.len();

        let key_height = KeyHeight {
            height,
            size,
            digest,
        };
        let key_size = KeySize {
            size,
            height,
            digest,
        };
        let key_digest = KeyDigest { height, digest };

        assert!(
            self.forks_by_height.remove(&key_height),
            "Missing mapping in forks_by_height: {key_height:#?}"
        );
        assert!(
            self.forks_by_size.remove(&key_size),
            "Missing mapping in forks_by_size: {key_size:#?}"
        );
        assert!(
            self.quorums_by_height.remove(&key_height),
            "Missing mapping in quorums_by_height: {key_height:#?}"
        );
        assert!(
            self.votes_invalid.insert(key_digest),
            "Duplicate mapping in votes_invalid: {key_digest:#?}"
        );

        for attestor in vote.signers {
            let key_vote = KeyVote { height, attestor };
            self.votes
                .remove(&key_vote)
                .expect("Missing mapping in votes_valid");
        }

        self.forks_best = self.find_best();
    }

    fn split_off(&mut self, height: common::types::Height) {
        let split = height.saturating_add(1);
        let digest_min = CompoundDigest::min();
        let attestor_min = attestor_primitives::AttestorId::from_public([0; 32]);

        let key_height = KeyHeight {
            height: split,
            size: 0,
            digest: digest_min,
        };
        let key_digest = KeyDigest {
            height: split,
            digest: digest_min,
        };
        let key_vote = KeyVote {
            height: split,
            attestor: attestor_min,
        };
        let key_height_pending = KeyHeightPending {
            height: split,
            digest: digest_min,
            prev_digest_tail: PrevDigestTail(attestor_primitives::Digest::zero()),
        };

        let after_by_height = self.forks_by_height.split_off(&key_height);
        let removed_by_height = std::mem::replace(&mut self.forks_by_height, after_by_height);

        for KeyHeight {
            digest,
            height,
            size,
        } in removed_by_height
        {
            let key_size = KeySize {
                size,
                height,
                digest,
            };

            assert!(
                self.forks_by_size.remove(&key_size),
                "Missing mapping in forks_by_size: {key_size:#?}"
            );

            self.forks_by_digest
                .remove(&digest)
                .expect("Missing mapping in forks_by_digest");
        }

        let after_pending = self.pending_by_height.split_off(&key_height_pending);
        let removed_pending = std::mem::replace(&mut self.pending_by_height, after_pending);

        for KeyHeightPending {
            height,
            digest,
            prev_digest_tail,
        } in removed_pending
        {
            let key_digest_pending = KeyDigestPending {
                height,
                digest,
                prev_digest_tail,
            };
            let key_tail_pending = KeyTailPending {
                prev_digest_tail,
                height,
                digest,
            };

            assert!(
                self.pending_by_prev_digest_tail.remove(&key_tail_pending),
                "Missing mapping in pending_by_prev_digest_tail: {key_tail_pending:#?}"
            );

            self.pending_by_digest
                .remove(&key_digest_pending)
                .expect("Missing mapping in pending_by_digest");
        }

        let after_quorums = self.quorums_by_height.split_off(&key_height);
        let _removed_quorums = std::mem::replace(&mut self.quorums_by_height, after_quorums);

        let after_invalid = self.votes_invalid.split_off(&key_digest);
        let _removed_invalid = std::mem::replace(&mut self.votes_invalid, after_invalid);

        let after_valid = self.votes.split_off(&key_vote);
        let _removed_valid = std::mem::replace(&mut self.votes, after_valid);

        // assert_eq!(
        //     self.votes_valid.len(),
        //     self.forks_by_digest.len(),
        //     "Invalid forks_by_digest length"
        // );
        // assert_eq!(
        //     self.forks_by_height.len(),
        //     self.forks_by_size.len(),
        //     "Invalid forks_by_size length"
        // );
        // assert!(
        //     self.votes_valid.len() >= self.quorums_by_height.len(),
        //     "Invalid quorums_by_height length"
        // );
    }

    fn find_best(&self) -> Option<AttestationVote> {
        self.quorums_by_height
            .first()
            .map(|KeyHeight { digest, .. }| digest)
            .or_else(|| {
                self.forks_by_size
                    .last()
                    .map(|KeySize { digest, .. }| digest)
            })
            .map(|digest| {
                self.forks_by_digest
                    .get(digest)
                    .expect("Missing mapping in forks_by_digest")
                    .clone()
            })
    }

    fn note_attestation_finalization(
        &mut self,
        info: stream::util::AttestationInfo,
    ) -> Result<(), Error> {
        tracing::debug!("Updating forks");

        self.split_off(info.height);
        self.last_finalized_digest = Some(info.digest);
        self.last_finalized_height = Some(info.height);

        let key_start = KeyTailPending {
            prev_digest_tail: PrevDigestTail(info.digest),
            height: info.height,
            digest: CompoundDigest::min(),
        };

        let key_stop = KeyTailPending {
            prev_digest_tail: PrevDigestTail(info.digest),
            height: common::types::Height::MAX,
            digest: CompoundDigest::max(),
        };
        let index = (
            std::ops::Bound::Included(key_start),
            std::ops::Bound::Included(key_stop),
        );
        let keys = self
            .pending_by_prev_digest_tail
            .range(index)
            .copied()
            .collect::<Vec<_>>();

        for KeyTailPending {
            prev_digest_tail,
            height,
            digest,
        } in keys
        {
            let key_digest_pending = KeyDigestPending {
                height,
                digest,
                prev_digest_tail,
            };
            let key_tail_pending = KeyTailPending {
                prev_digest_tail,
                height,
                digest,
            };
            let key_height_pending = KeyHeightPending {
                height,
                digest,
                prev_digest_tail,
            };

            assert!(
                self.pending_by_prev_digest_tail.remove(&key_tail_pending),
                "Missing mapping in pending_by_prev_digest_tail: {key_tail_pending:#?}"
            );
            assert!(
                self.pending_by_height.remove(&key_height_pending),
                "Missing mapping in pending_by_height: {key_height_pending:#?}"
            );

            let vote = self
                .pending_by_digest
                .remove(&key_digest_pending)
                .expect("Missing mapping in pending_by_digest");

            for attestation in vote.votes {
                let key_vote = KeyVote {
                    height,
                    attestor: attestation.attestor.clone(),
                };

                self.votes
                    .remove(&key_vote)
                    .expect("Missing mapping in votes");

                self.push(attestation)?;
            }
        }

        self.forks_best = self.find_best();

        Ok(())
    }

    fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
    ) {
        tracing::debug!("Updating forks");

        self.forks_by_digest.clear();
        self.forks_by_height.clear();
        self.forks_by_size.clear();
        self.forks_best = None;

        self.pending_by_digest.clear();
        self.pending_by_prev_digest_tail.clear();
        self.pending_by_height.clear();

        self.votes.clear();
        self.votes_invalid.clear();

        self.quorums_by_height.clear();

        self.validate_quorum
            .note_attestation_interval_change(interval_new);
    }

    fn note_attestation_chain_reversion(&mut self, info: stream::util::AttestationInfo) {
        tracing::debug!("Clearing forks");

        self.forks_by_digest.clear();
        self.forks_by_height.clear();
        self.forks_by_size.clear();
        self.forks_best = None;

        self.pending_by_digest.clear();
        self.pending_by_prev_digest_tail.clear();
        self.pending_by_height.clear();

        self.votes.clear();
        self.votes_invalid.clear();

        self.quorums_by_height.clear();

        self.last_finalized_digest = Some(info.digest);
        self.last_finalized_height = Some(info.height);
    }
}

struct AttestationPoolValid {
    quorums_valid: std::collections::BTreeMap<
        common::types::Height,
        (
            common::types::AttestationSigned,
            Vec<common::types::Attestation>,
        ),
    >,
}

impl AttestationPoolValid {
    fn new() -> Self {
        Self {
            quorums_valid: Default::default(),
        }
    }

    fn push(
        &mut self,
        signed: common::types::AttestationSigned,
        votes: Vec<common::types::Attestation>,
    ) {
        let height = signed.attestation.header_number();
        self.quorums_valid.insert(height, (signed, votes));
    }

    #[allow(clippy::type_complexity)]
    fn pop(
        &mut self,
    ) -> Option<(
        common::types::Height,
        attestor_primitives::Digest,
        cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
        Vec<common::types::Attestation>,
    )> {
        self.quorums_valid
            .pop_last()
            .map(|(_height, (att, votes))| (att.header_number(), att.digest(), att.into(), votes))
    }

    fn note_attestation_finalization(&mut self, info: stream::util::AttestationInfo) {
        tracing::debug!("Updating known quorums");

        let split = info.height.saturating_add(1);
        let after = self.quorums_valid.split_off(&split);
        let _removed = std::mem::replace(&mut self.quorums_valid, after);
    }

    fn note_attestation_chain_reversion(&mut self) {
        self.quorums_valid.clear();
    }

    fn note_attestation_interval_change(&mut self) {
        self.quorums_valid.clear();
    }
}

struct AttestationPoolDelays {
    time: std::collections::BTreeMap<common::types::Height, std::time::Instant>,
    metrics: common::types::Metrics,
}

impl AttestationPoolDelays {
    fn new(metrics: common::types::Metrics) -> Self {
        Self {
            time: Default::default(),
            metrics,
        }
    }

    fn push(&mut self, height: common::types::Height) {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.time.entry(height) {
            entry.insert(std::time::Instant::now());
        }
    }

    fn pop(&mut self, height: common::types::Height) -> Option<std::time::Duration> {
        self.time.remove(&height).map(|then| then.elapsed())
    }

    fn note_attestation_finalization(&mut self, info: stream::util::AttestationInfo) {
        tracing::debug!("Updating quorum delays");
        let mut removed = self.time.split_off(&(info.height.saturating_add(1)));
        std::mem::swap(&mut removed, &mut self.time);

        for (_, then) in removed {
            self.metrics
                .update_attestation_delay_finalization(then.elapsed());
        }
    }

    fn note_attestation_interval_change(&mut self) {
        tracing::debug!("Updating quorum delays");
        self.time.clear();
    }

    fn note_attestation_chain_reversion(&mut self) {
        tracing::debug!("Updating quorum delays");
        self.time.clear();
    }
}

// ----------------------------------- [ Attestation Sender ] ---------------------------------- //

impl AttestationPoolSender {
    /// Sends an attestation to the attestation pool. Returns [`None`] if the pool has been
    /// [`closed`].
    ///
    /// [`closed`]: Self::close
    #[tracing::instrument(
        skip_all,
        fields(
            digest = ?attestation.digest(),
            height = attestation.header_number()
        )
    )]
    pub fn send(&self, attestation: common::types::Attestation) -> Option<Result<(), Error>> {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            Some(inner.push(attestation))
        } else {
            None
        }
    }

    /// Closes the attestation pool. Successive calls to [`send`] will return [`None`], while
    /// polling via the [`receiver`] end will terminate its [`Stream`].
    ///
    /// [`send`]: Self::send
    /// [`receiver`]: AttestationPoolReceiver
    /// [`Stream`]: futures::Stream
    #[allow(unused)]
    pub fn close(self) {
        *self.common.pool.lock() = AttestationPool::Closed;
    }

    pub fn note_target_sample_size_change(&self, target_sample_size_new: u32) {
        let threshold = attestor_primitives::calculate_threshold(target_sample_size_new) as usize;
        let quorum_new = std::num::NonZeroUsize::new(threshold);

        let Some(quorum_new) = quorum_new else {
            return;
        };

        let AttestationPool::Open(inner) = &mut *self.common.pool.lock() else {
            return;
        };

        inner.forks.validate_quorum.target_quorum = quorum_new;

        if let Some(waker) = inner.wakers.pop_back() {
            tracing::debug!("Target sample size updated, waking receiver...");
            waker.wake();
        };
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl AttestationPoolSender {
    /// A new attestation has reached finality on the execution chain.
    ///
    /// Remove all attestations _up to and including_ that attestation height from the inner
    /// attestation pool.
    #[tracing::instrument(
        skip_all,
        fields(digest = ?info.digest, height = info.height),
        level = "debug"
    )]
    pub fn note_attestation_finalization(
        &mut self,
        info: stream::util::AttestationInfo,
    ) -> Result<(), Error> {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            // Remove past quorums
            inner.valid.note_attestation_finalization(info);

            // Update metrics
            inner.attestation_delay.note_attestation_finalization(info);

            // Updating the inner pool
            inner.forks.note_attestation_finalization(info)?;
        }

        Ok(())
    }

    /// A new attestation interval has been set on-chain.
    //
    // Clear the attestation pool and update the target height and locally tracked attestation
    // interval.
    #[tracing::instrument(
        skip_all,
        fields(interval = interval_new),
        level = "debug"
    )]
    pub fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
    ) {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            inner.digest_local = None;
            // Updating the inner pool
            inner.forks.note_attestation_interval_change(interval_new);

            // Updating quorums
            inner.valid.note_attestation_interval_change();

            // Update metrics
            inner.attestation_delay.note_attestation_interval_change();
        }
    }

    #[tracing::instrument(
        skip_all,
        fields(
            attestors = attestors
                .iter()
                .map(ToString::to_string)
                .reduce(|mut a, b| {
                    a.reserve(b.len() + 1);
                    a.push_str(", ");
                    a.push_str(&b);
                    a
                })
                .unwrap_or_default()
        )
        level = "debug"
    )]
    pub fn note_attestors_elected(&mut self, attestors: Vec<cc_client::AccountId32>) {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            tracing::warn!("🗂️ Updating the attestor set");
            inner.validate_attestor = ValidateAttestor::new(attestors);
        }
    }

    /// An attestation chain reversion has been detected.
    /// We need to clear the structures `forks`, `valid`, and `attestation_delay`
    #[tracing::instrument(
        skip_all,
        fields(digest = ?info.digest, height = info.height),
        level = "debug"
    )]
    pub fn note_attestation_chain_reversion(&mut self, info: stream::util::AttestationInfo) {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            // Clear digest local, as it no longer tracks a valid new attestation
            inner.digest_local = None;
            // Updating the inner pool
            inner.forks.note_attestation_chain_reversion(info);

            // Remove past quorums
            inner.valid.note_attestation_chain_reversion();

            // Update metrics
            inner.attestation_delay.note_attestation_chain_reversion();
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
    /// Closes the attestation pool. Successive calls to [`send`] will return [`None`], while the
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
    #[tracing::instrument(skip_all, fields(%permit))]
    pub fn mark_valid(&self, permit: Permit) {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::debug!("Removing valid attestation");
                inner.mark_valid(permit);
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to remove valid attestation from pool after it has been closed"
                );
            }
        }
    }

    /// Marks an attestation as **invalid**, causing it to be removed from the attestation pool. The
    /// pool's target height _is not_ updated.
    #[tracing::instrument(skip_all, fields(%permit))]
    pub fn mark_invalid(&self, permit: Permit) {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::debug!("Removing invalid attestation");
                inner.mark_invalid(permit);
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to remove invalid attestation from pool after it has been closed"
                );
            }
        }
    }

    /// Marks an attestation as valid and updates the pool's target height. Contrarily to
    /// [`mark_valid`], this method does _not_ remove the attestation from the pool and instead
    /// moves it to an unbounded pending queue for future submission by the [validation worker].
    /// Pending validated attestations can be retrieved with [`take_next_validated`].
    ///
    /// [`mark_valid`]: Self::mark_valid
    /// [validation worker]: crate::worker::validation
    /// [`take_next_validated`]: Self::take_next_validated
    #[tracing::instrument(skip_all, fields(%permit))]
    pub fn mark_for_later(
        &self,
        permit: Permit,
        signed: common::types::AttestationSigned,
        votes: Vec<common::types::Attestation>,
    ) {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::debug!("Marking attestation for later removal");
                inner.mark_for_later(permit, signed, votes);
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to remove valid attestation from pool after it has been closed"
                );
            }
        }
    }

    /// Retrieves the next pending validated attestation marked with [`mark_for_later`] to submit
    /// it to the runtime.
    ///
    /// Returns:
    ///
    /// [`None`] if no pending validated attestation is available, can happen if there was not
    /// enough time to validate attestations between submissions.
    ///
    /// [`mark_for_later`]: Self::mark_for_later
    #[allow(clippy::type_complexity)]
    pub fn take_next_validated(
        &self,
    ) -> Option<(
        common::types::Height,
        attestor_primitives::Digest,
        cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
        Vec<common::types::Attestation>,
    )> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::debug!("Checking for next validated attestation");
                inner.valid.pop()
            }
            AttestationPool::Closed => {
                tracing::warn!(
                    "Tried to take attestation batch from pool after it has been closed"
                );
                None
            }
        }
    }
}

impl futures::Stream for AttestationPoolReceiver {
    type Item = (Quorum, Permit, Option<cc_client::H256>);

    /// This future is cancellation-safe, as it does not perform any mutations on the inner pool.
    #[tracing::instrument(skip_all)]
    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => match inner.peek() {
                Some((quorum, permit)) => {
                    tracing::debug!(height = quorum.header_number(), "Found a quorum");
                    std::task::Poll::Ready(Some((quorum, permit, inner.digest_local)))
                }
                None => {
                    tracing::debug!("No quorum found, waiting for new attestations...");
                    inner.wakers.push_front(cx.waker().clone());
                    std::task::Poll::Pending
                }
            },
            AttestationPool::Closed => std::task::Poll::Ready(None),
        }
    }
}

// --------------------------------- [ Attestation Internals ] --------------------------------- //

#[derive(Clone, Debug)]
struct AttestationVote {
    attestation: common::types::Attestation,
    votes: Vec<common::types::Attestation>,
    signers: std::collections::HashSet<attestor_primitives::AttestorId>,
}

impl AttestationVote {
    fn new(attestation: common::types::Attestation) -> Self {
        Self {
            votes: vec![attestation.clone()],
            signers: std::collections::HashSet::from([attestation.attestor.clone()]),
            attestation,
        }
    }

    fn update(&mut self, mut vote: AttestationVote) {
        std::mem::swap(&mut self.votes, &mut vote.votes); // Preserves insertion order

        self.signers.extend(vote.signers);
        self.votes.extend(vote.votes);

        assert_eq!(
            self.votes.len(),
            self.signers.len(),
            "Vote count does not match attestor count"
        );
    }
}

impl std::cmp::PartialEq for AttestationVote {
    fn eq(&self, other: &Self) -> bool {
        // Attestation header number is implied in the digest computation and so does not need to
        // be checked manually as changing it would result in a different digest. The header hash
        // is NOT part of digest computation however and needs to be checked manually.
        self.attestation.digest() == other.attestation.digest()
            && self.attestation.attestation_data.header_hash
                == other.attestation.attestation_data.header_hash
    }
}

impl std::cmp::Eq for AttestationVote {}

impl std::hash::Hash for AttestationVote {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.attestation.digest().hash(state);
        self.attestation.attestation_data.header_hash.hash(state);
    }
}

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

    pub fn votes(&self) -> Vec<common::types::Attestation> {
        self.0.clone()
    }
}

/// A unique permit which can be used to remove attestation from the attestation pool via
/// [`mark_valid`], [`mark_for_later`] and [`mark_invalid`].
///
/// [`mark_valid`]: AttestationPoolReceiver::mark_valid
/// [`mark_for_later`]: AttestationPoolReceiver::mark_for_later
/// [`mark_invalid`]: AttestationPoolReceiver::mark_invalid
#[must_use]
#[derive(Debug, PartialEq, Eq)]
pub struct Permit(CompoundInfo);

// ------------------------------------ [ Quorum Validation ] ---------------------------------- //

/// Encapsulates quorum information to check if an attestation is ready for polling.
///
/// An attestation is ready for polling when enough attestors have voted for it and its height is
/// next in line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidateQuorum {
    target_quorum: std::num::NonZeroUsize,
    attestation_interval: std::num::NonZero<common::types::Height>,
    start_height: common::types::Height,
    max_catchup: std::num::NonZero<common::types::Height>,
}

impl ValidateQuorum {
    pub const fn new(
        target_quorum: std::num::NonZeroUsize,
        attestation_interval: std::num::NonZero<common::types::Height>,
        start_height: common::types::Height,
        max_catchup: std::num::NonZero<common::types::Height>,
    ) -> Self {
        Self {
            target_quorum,
            attestation_interval,
            start_height,
            max_catchup,
        }
    }

    #[tracing::instrument(skip_all, fields(target_quorum = %self.target_quorum))]
    fn validate(&self, attestation: &AttestationVote) -> bool {
        tracing::debug!(
            height = attestation.attestation.header_number(),
            quorum = attestation.signers.len(),
            "Validating attestation"
        );

        attestation.signers.len() >= self.target_quorum.into()
    }

    /// Validates that a vote height is admissible: aligned with the attestation interval,
    /// above the latest finalized height, and within the maximum catch-up window.
    fn validate_height(
        &self,
        height: common::types::Height,
        last_finalized_height: Option<common::types::Height>,
    ) -> bool {
        let catchup_window = self
            .max_catchup
            .get()
            .saturating_mul(self.attestation_interval.get());

        let base = last_finalized_height.unwrap_or(self.start_height);
        let upper_bound = base.saturating_add(catchup_window);

        // Must be above the last finalized height (if any), or at/above start_height
        let above_finalized = match last_finalized_height {
            Some(finalized) => height > finalized,
            None => height >= self.start_height,
        };

        above_finalized
            && height >= self.start_height
            && (height - self.start_height) % self.attestation_interval.get() == 0
            && height <= upper_bound
    }

    fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
    ) {
        tracing::debug!("Updating quorum validation");
        self.attestation_interval = interval_new;
    }
}

// ----------------------------------- [ Attestor Validation ] --------------------------------- //

/// Enforces permissioned attesting.
///
/// Attestors are retrieved on-chain from the currently elected authorities. Any other attestation
/// source is denied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValidateAttestor {
    attestor_set: std::collections::HashSet<attestor_primitives::AttestorId>,
}

impl ValidateAttestor {
    pub fn new(attestors: Vec<cc_client::AccountId32>) -> Self {
        Self {
            attestor_set: attestors
                .into_iter()
                .map(|attestor| {
                    attestor_primitives::AttestorId::new(sp_core::crypto::AccountId32::new(
                        attestor.0,
                    ))
                })
                .collect(),
        }
    }

    pub fn attestors(&self) -> &std::collections::HashSet<attestor_primitives::AttestorId> {
        &self.attestor_set
    }
}

impl std::fmt::Display for ValidateAttestor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Permissioned: {:?}", self.attestor_set)
    }
}

impl ValidateAttestor {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), Error> {
        if !self.attestor_set.contains(&attestation.attestor) {
            return Err(Error::Unauthorized(
                attestation.attestor.clone(),
                attestation.header_number(),
            ));
        }
        Ok(())
    }
}

// ----------------------------------------- [ Display ] --------------------------------------- //

impl std::fmt::Display for ValidateQuorum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ vote_count: {} }}", self.target_quorum)
    }
}

impl std::fmt::Display for Permit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{{ height: {}, digest: {} }}",
            self.0.height, self.0.digest
        )
    }
}

// ---------------------------------------- [ Fixtures ] --------------------------------------- //

#[cfg(test)]
mod constants {
    use super::*;

    pub const ATTESTOR_VALID_0: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_0________________");
    pub const ATTESTOR_VALID_1: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_1________________");
    pub const ATTESTOR_VALID_2: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_2________________");
    pub const ATTESTOR_VALID_3: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_valid_3________________");
    pub const ATTESTOR_INVALID: attestor_primitives::AttestorId =
        attestor_primitives::AttestorId::from_public(*b"attestor_invalid________________");

    pub const DIGEST_0: CompoundDigest = CompoundDigest {
        digest: sp_core::H256(*b"digest_0________________________"),
        digest_continuity: sp_core::H256(*b"digest_0________________________"),
        header_hash: attestor_primitives::Digest::zero(),
    };
    pub const DIGEST_1: CompoundDigest = CompoundDigest {
        digest: sp_core::H256(*b"digest_1________________________"),
        digest_continuity: sp_core::H256(*b"digest_0________________________"),
        header_hash: attestor_primitives::Digest::zero(),
    };

    pub const TIMEOUT: std::time::Duration = std::time::Duration::from_millis(10);
}

#[cfg(test)]
mod fixtures {
    use super::*;
    use constants::*;

    #[rstest::fixture]
    pub fn attestation(
        #[default([ATTESTOR_VALID_0])] attestors: impl IntoIterator<
            Item = attestor_primitives::AttestorId,
        >,
        #[default(2)] header_number: common::types::Height,
        #[default(DIGEST_0)] prev_digest: CompoundDigest,
        #[default(DIGEST_0)] header_hash: CompoundDigest,
    ) -> AttestationVote {
        let mut iter = attestors.into_iter();

        let attestation =
            move |attestor: attestor_primitives::AttestorId| -> common::types::Attestation {
                common::types::Attestation {
                    attestation_data: attestor_primitives::AttestationData {
                        header_number,
                        prev_digest: Some(prev_digest.digest),
                        header_hash: header_hash.digest,
                        ..Default::default()
                    },
                    attestor,
                    signature: Default::default(),
                    signature_bls: attestor_primitives::bls::WrapEncode(
                        bls_signatures::PrivateKey::new(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                            .sign(b"0xdeadbeef"),
                    ),
                    continuity_proof: attestor_primitives::block::ContinuityProof::new(
                        prev_digest.digest,
                        vec![attestor_primitives::Digest::default()],
                    ),
                }
            };

        let attestor = iter.next().unwrap();
        iter.fold(
            AttestationVote {
                votes: vec![attestation(attestor.clone())],
                signers: std::collections::HashSet::from([attestor.clone()]),
                attestation: attestation(attestor),
            },
            |mut vote, attestor| {
                vote.votes.push(attestation(attestor.clone()));
                vote.signers.insert(attestor);
                vote
            },
        )
    }

    #[rstest::fixture]
    pub fn attestation_signed(attestation: AttestationVote) -> common::types::AttestationSigned {
        attestor_primitives::SignedAttestation {
            attestation: attestation.attestation.attestation_data,
            signature: [0u8; 96],
            attestors: attestation
                .votes
                .iter()
                .map(|att| att.attestor.clone())
                .collect(),
            continuity_proof: attestation.attestation.continuity_proof,
        }
    }

    #[rstest::fixture]
    pub fn quorum(
        #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
            + Clone,
        #[default(2)] _header_number: common::types::Height,
        #[default(DIGEST_0)] _prev_digest: CompoundDigest,
        #[default(DIGEST_0)] _header_hash: CompoundDigest,
        #[with(_attestors.clone(), _header_number, _prev_digest, _header_hash)]
        attestation: AttestationVote,
    ) -> Quorum {
        Quorum(attestation.votes)
    }

    #[rstest::fixture]
    pub fn validate_quorum(#[default(2)] vote_count: usize) -> ValidateQuorum {
        ValidateQuorum {
            target_quorum: vote_count.try_into().unwrap(),
            start_height: 1,
            attestation_interval: std::num::NonZero::<common::types::Height>::MIN,
            max_catchup: crate::common::constants::MAX_CATCHUP,
        }
    }

    #[rstest::fixture]
    pub fn validate_attestor(
        #[default([ATTESTOR_VALID_0, ATTESTOR_VALID_1, ATTESTOR_VALID_2, ATTESTOR_VALID_3])]
        attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>,
    ) -> ValidateAttestor {
        ValidateAttestor {
            attestor_set: attestors.into_iter().collect(),
        }
    }

    #[rstest::fixture]
    pub fn attestors(
        #[default([ATTESTOR_VALID_0, ATTESTOR_VALID_1, ATTESTOR_VALID_2, ATTESTOR_VALID_3])]
        attestor_set: impl IntoIterator<Item = attestor_primitives::AttestorId>,
    ) -> Vec<cc_client::AccountId32> {
        attestor_set
            .into_iter()
            .map(|attestor| cc_client::AccountId32(attestor.public_key()))
            .collect()
    }

    #[rstest::fixture]
    pub fn metrics() -> common::types::Metrics {
        let config = crate::worker::api::metrics::ConfigBuilder::new()
            .with_name("test")
            .with_address(cc_client::AccountId32([0; 32]))
            .with_peer_id(libp2p::PeerId::random())
            .with_chain_key(2u64)
            .with_start_height(common::types::Height::MIN)
            .with_start_attestation(None)
            .with_genesis(common::types::Height::MIN)
            .with_attestation_latest_eth(common::types::Height::MIN)
            .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
            .build();
        std::sync::Arc::new(crate::worker::api::metrics::Metrics::new(config))
    }

    #[rstest::fixture]
    pub fn config(
        validate_quorum: ValidateQuorum,
        attestors: Vec<cc_client::AccountId32>,
        metrics: common::types::Metrics,
    ) -> Config {
        ConfigBuilder::new()
            .with_attestors(attestors)
            .with_quorum(validate_quorum.target_quorum)
            .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
            .with_start_attestation(Some(stream::util::AttestationInfo {
                digest: DIGEST_0.digest,
                height: common::types::Height::MIN,
            }))
            .with_start_height(1u64)
            .with_max_catchup(crate::common::constants::MAX_CATCHUP)
            .with_metrics(metrics)
            .build()
    }

    #[rstest::fixture]
    pub fn permit(
        #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
            + Clone,
        #[default(2)] _header_number: common::types::Height,
        #[default(DIGEST_0)] _prev_digest: CompoundDigest,
        #[default(DIGEST_0)] _header_hash: CompoundDigest,
        #[with(_attestors.clone(), _header_number, _prev_digest, _header_hash)]
        attestation: AttestationVote,
    ) -> Permit {
        let height_prev = attestation
            .attestation
            .header_number()
            .checked_sub(
                attestation.attestation.continuity_proof.len() as common::types::Height + 1,
            )
            .unwrap();
        let digest_continuity = attestation
            .attestation
            .continuity_proof
            .compute_continuity_digest(height_prev);

        Permit(CompoundInfo {
            height: attestation.attestation.header_number(),
            digest: attestation.attestation.digest(),
            digest_continuity,
            header_hash: attestation.attestation.attestation_data.header_hash,
        })
    }
}

// -------------------------------------- [ Sanity Checks ] ------------------------------------ //

#[cfg(test)]
mod test {
    use crate::common::fixtures::*;

    use super::constants::*;
    use super::fixtures::*;
    use super::*;

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_valid(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2, DIGEST_0)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 2, DIGEST_0)]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_2], 2, DIGEST_1)]
        attestation_2: AttestationVote,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 2, DIGEST_0)]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_2.attestation.clone()).unwrap().is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);

        rx.mark_valid(permit);

        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert!(!inner.forks.forks_by_height.contains(&KeyHeight {
            height: 1,
            size: 2,
            digest: DIGEST_0
        }));
        assert_eq!(
            inner.digest_local,
            Some(cc_client::H256(attestation_1.attestation.digest().0))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_invalid(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationVote,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);
        rx.mark_invalid(permit);

        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        let height_prev = attestation_0.attestation.header_number()
            - attestation_0.attestation.continuity_proof.len() as common::types::Height
            - 1;
        let digest_continuity = attestation_0
            .attestation
            .continuity_proof
            .compute_continuity_digest(height_prev);

        assert!(inner.forks.votes_invalid.contains(&KeyDigest {
            height: attestation_0.attestation.header_number(),
            digest: CompoundDigest {
                digest: attestation_0.attestation.digest(),
                digest_continuity,
                header_hash: attestation_0.attestation.attestation_data.header_hash
            }
        }));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_deduplicate_header_hash(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2, DIGEST_0, DIGEST_0)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 2, DIGEST_0, DIGEST_1)]
        attestation_1: AttestationVote,
        config: Config,
    ) {
        let (sx, rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        let height_prev = attestation_0.attestation.header_number()
            - attestation_0.attestation.continuity_proof.len() as common::types::Height
            - 1;
        let digest_continuity_0 = attestation_0
            .attestation
            .continuity_proof
            .compute_continuity_digest(height_prev);

        let height_prev = attestation_1.attestation.header_number()
            - attestation_1.attestation.continuity_proof.len() as common::types::Height
            - 1;
        let digest_continuity_1 = attestation_1
            .attestation
            .continuity_proof
            .compute_continuity_digest(height_prev);

        assert_eq!(inner.forks.votes.len(), 2);
        assert_eq!(inner.forks.forks_by_digest.len(), 2);
        assert_eq!(inner.forks.forks_by_size.len(), 2);

        assert_eq!(
            inner
                .forks
                .forks_by_digest
                .get(&CompoundDigest {
                    digest: attestation_0.attestation.digest(),
                    digest_continuity: digest_continuity_0,
                    header_hash: attestation_0.attestation.attestation_data.header_hash
                })
                .unwrap(),
            &attestation_0
        );

        assert_eq!(
            inner
                .forks
                .forks_by_digest
                .get(&CompoundDigest {
                    digest: attestation_1.attestation.digest(),
                    digest_continuity: digest_continuity_1,
                    header_hash: attestation_1.attestation.attestation_data.header_hash
                })
                .unwrap(),
            &attestation_1
        );

        assert!(inner.forks.forks_by_size.contains(&KeySize {
            size: 1,
            height: 2,
            digest: CompoundDigest {
                digest: attestation_0.attestation.digest(),
                digest_continuity: digest_continuity_0,
                header_hash: attestation_0.attestation.attestation_data.header_hash
            }
        }));

        assert!(inner.forks.forks_by_size.contains(&KeySize {
            size: 1,
            height: 2,
            digest: CompoundDigest {
                digest: attestation_1.attestation.digest(),
                digest_continuity: digest_continuity_1,
                header_hash: attestation_1.attestation.attestation_data.header_hash
            }
        }));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_mark_for_later(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationVote,
        #[from(attestation_signed)] attestation_signed: common::types::AttestationSigned,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert_matches::assert_matches!(rx.take_next_validated(), None);

        assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);
        rx.mark_for_later(
            permit,
            attestation_signed.clone(),
            vec![
                attestation_0.attestation.clone(),
                attestation_1.attestation.clone(),
            ],
        );

        // Such types, much wow... -fuck subxt and the incompatible dependencies which make using
        // our own types an even more royal pain $$%%^#$#
        let attestation_expected: cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        > = attestation_signed.clone().into();

        assert_matches::assert_matches!(rx.take_next_validated(), Some((height, digest, attestation, votes)) => {
            assert_eq!(height, attestation_0.attestation.header_number());
            assert_eq!(digest, attestation_0.attestation.digest());
            // Other types in this don't implement PartialEq and Eq...
            assert_eq!(attestation.attestors, attestation_expected.attestors);
            assert_eq!(votes,
                vec![
                    attestation_0.attestation,
                    attestation_1.attestation,
                ],
            );
        });

        assert_eq!(
            sx.common.pool.lock().expect_open().digest_local,
            Some(cc_client::H256(attestation_signed.digest().0))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_pending(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2, DIGEST_1)]
        attestation_pending: AttestationVote,
        config: Config,
    ) {
        let (mut sx, rx) = attestation_pool(config);

        assert!(sx
            .send(attestation_pending.attestation.clone())
            .unwrap()
            .is_ok());

        {
            let mut pool = rx.common.pool.lock();
            let inner = pool.expect_open();

            let height_prev = attestation_pending.attestation.header_number()
                - attestation_pending.attestation.continuity_proof.len() as common::types::Height
                - 1;
            let digest_continuity = attestation_pending
                .attestation
                .continuity_proof
                .compute_continuity_digest(height_prev);

            assert_eq!(inner.forks.pending_by_digest.len(), 1);
            assert_eq!(inner.forks.pending_by_prev_digest_tail.len(), 1);
            assert_eq!(inner.forks.pending_by_height.len(), 1);
            assert!(inner
                .forks
                .pending_by_prev_digest_tail
                .contains(&KeyTailPending {
                    prev_digest_tail: PrevDigestTail(DIGEST_1.digest),
                    height: 2,
                    digest: CompoundDigest {
                        digest: attestation_pending.attestation.digest(),
                        digest_continuity,
                        header_hash: attestation_pending.attestation.attestation_data.header_hash
                    },
                }));
        }

        sx.note_attestation_finalization(stream::util::AttestationInfo {
            digest: DIGEST_1.digest,
            height: 1,
        })
        .unwrap();

        {
            let mut pool = rx.common.pool.lock();
            let inner = pool.expect_open();
            let vote = AttestationVote::new(attestation_pending.attestation.clone());

            assert_eq!(inner.forks.forks_best.clone().unwrap(), vote);
        }
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_err_invalid_attestor(
        #[with([ATTESTOR_INVALID])] attestation: AttestationVote,
        config: Config,
    ) {
        let (sx, _rx) = attestation_pool(config);

        assert_matches::assert_matches!(
            sx.send(attestation.attestation.clone()),
            Some(Err(Error::Unauthorized(ATTESTOR_INVALID, 2)))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_async_wake_receiver(
        _logs: (),
        #[with([ATTESTOR_VALID_0])] attestation: AttestationVote,
        #[with([ATTESTOR_VALID_0])] permit: Permit,
        #[with([ATTESTOR_VALID_0])] quorum: Quorum,
        #[from(validate_quorum)]
        #[with(1)]
        _quorum_validate: ValidateQuorum,
        #[with(_quorum_validate.clone())] config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);
        let mut fut = tokio_test::task::spawn(rx.next());

        tokio_test::assert_pending!(fut.poll());
        assert!(sx.send(attestation.attestation.clone()).unwrap().is_ok());
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
    async fn attestation_pool_quorum_basic(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationVote,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        quorum: Quorum,
        #[from(permit)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        permit: Permit,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());

        let actual = rx.next().await;
        let expected = Some((quorum, permit, None));

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    #[allow(clippy::too_many_arguments)]
    async fn attestation_pool_quorum_highest(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1])]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_2])]
        attestation_2: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 101)]
        attestation_3: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 101)]
        attestation_4: AttestationVote,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 101)]
        quorum: Quorum,
        #[from(permit)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 101)]
        permit: Permit,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        // Source chain height 1 (default)
        assert!(sx.send(attestation_0.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_2.attestation.clone()).unwrap().is_ok());

        // Source chain height 101
        assert!(sx.send(attestation_3.attestation.clone()).unwrap().is_ok());
        assert!(sx.send(attestation_4.attestation.clone()).unwrap().is_ok());

        // NOTE: even though quorum 1 has LESS votes, it still passes the quorum threshold of 2.
        // The attestation pool always favors the HIGHEST quorum so as to improve catchup speed.

        let actual = rx.next().await;
        let expected = Some((quorum, permit, None));

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_close_sender(
        _logs: (),
        #[with([ATTESTOR_VALID_1])] attestation: AttestationVote,
        config: Config,
    ) {
        let (sx, rx) = attestation_pool(config);
        rx.close();
        assert_matches::assert_matches!(sx.send(attestation.attestation.clone()), None);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_close_receiver(
        _logs: (),
        #[with([ATTESTOR_VALID_1])] attestation: AttestationVote,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);
        assert!(sx.send(attestation.attestation.clone()).unwrap().is_ok());

        sx.close();
        assert!(rx.next().await.is_none());
    }

    #[rstest::rstest]
    fn quorum_parameters_validate(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_1: AttestationVote,
        validate_quorum: ValidateQuorum,
    ) {
        assert!(validate_quorum.validate(&attestation_0));
        assert!(!validate_quorum.validate(&attestation_1));
    }

    #[rstest::rstest]
    fn validator_parameters_validate(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationVote,
        validate_attestor: ValidateAttestor,
    ) {
        assert!(validate_attestor
            .validate(&attestation_0.attestation)
            .is_ok());
        assert_matches::assert_matches!(
            validate_attestor.validate(&attestation_2.attestation),
            Err(Error::Unauthorized(ATTESTOR_INVALID, 2))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    #[allow(clippy::too_many_arguments)]
    async fn chain_reversion_resets_validation_pool(
        _logs: (),
        // Attestations that will be marked valid via mark_for_later
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2, DIGEST_0)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 2, DIGEST_0)]
        attestation_1: AttestationVote,
        // Attestations that will be marked invalid
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 3, DIGEST_0)]
        attestation_2: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 3, DIGEST_0)]
        attestation_3: AttestationVote,
        // Attestations that will remain in forks after removals
        #[from(attestation)]
        #[with([ATTESTOR_VALID_2], 4, DIGEST_0)]
        attestation_4: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_3], 4, DIGEST_0)]
        attestation_5: AttestationVote,
        // Attestation that will be entered into pending
        #[from(attestation)]
        #[with([ATTESTOR_VALID_2], 3, DIGEST_1)]
        attestation_pending: AttestationVote,
        #[from(validate_quorum)]
        #[with(2)]
        _quorum_validate: ValidateQuorum,
        #[from(config)]
        #[with(_quorum_validate.clone())]
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (mut sx, mut rx) = attestation_pool(config);

        // ------------------------------------------------------------------------
        // 1) Create a quorum and mark it for later.
        //    This populates:
        //      - valid.quorums_valid
        //      - digest_local
        // ------------------------------------------------------------------------
        assert!(sx
            .send(attestation_0.attestation.clone())
            .unwrap()
            .as_ref()
            .is_ok());
        assert!(sx
            .send(attestation_1.attestation.clone())
            .unwrap()
            .as_ref()
            .is_ok());

        let (_quorum_high, permit_0, _digest_local) = rx.next().await.unwrap();

        let attestation_signed_0 = common::types::AttestationSigned {
            attestation: attestation_0.attestation.attestation_data.clone(),
            signature: [0u8; 96],
            attestors: vec![
                attestation_0.attestation.attestor.clone(),
                attestation_1.attestation.attestor.clone(),
            ],
            continuity_proof: attestation_0.attestation.continuity_proof.clone(),
        };

        rx.mark_for_later(
            permit_0,
            attestation_signed_0,
            vec![
                attestation_0.attestation.clone(),
                attestation_1.attestation.clone(),
            ],
        );

        // ------------------------------------------------------------------------
        // 2) Create another quorum and mark it invalid.
        //    This populates votes_invalid.
        // ------------------------------------------------------------------------
        assert!(sx
            .send(attestation_2.attestation.clone())
            .unwrap()
            .as_ref()
            .is_ok());
        assert!(sx
            .send(attestation_3.attestation.clone())
            .unwrap()
            .as_ref()
            .is_ok());

        let (_quorum_low, permit_1, _digest_local) = rx.next().await.unwrap();
        rx.mark_invalid(permit_1);

        // ------------------------------------------------------------------------
        // 3) Create another quorum and leave it in forks.
        // This populates forks_by_digest / forks_by_height / forks_by_size / quorums_by_height / votes
        // ------------------------------------------------------------------------
        assert!(sx
            .send(attestation_4.attestation.clone())
            .unwrap()
            .as_ref()
            .is_ok());
        assert!(sx
            .send(attestation_5.attestation.clone())
            .unwrap()
            .as_ref()
            .is_ok());

        // ------------------------------------------------------------------------
        // 4) Add a pending attestation.
        //    This populates:
        //      - pending_by_digest / pending_by_prev_digest_tail / pending_by_height
        //      - attestation_delay.time
        // ------------------------------------------------------------------------
        assert!(sx
            .send(attestation_pending.attestation.clone())
            .unwrap()
            .is_ok());

        // Sanity-check that we actually populated the structures before reversion.
        {
            let mut pool = rx.common.pool.lock();
            let inner = pool.expect_open();

            assert!(inner.digest_local.is_some());

            assert!(!inner.forks.forks_by_digest.is_empty());
            assert!(!inner.forks.forks_by_height.is_empty());
            assert!(!inner.forks.forks_by_size.is_empty());
            assert!(inner.forks.forks_best.is_some());

            assert!(!inner.forks.pending_by_digest.is_empty());
            assert!(!inner.forks.pending_by_prev_digest_tail.is_empty());
            assert!(!inner.forks.pending_by_height.is_empty());

            assert!(!inner.forks.votes.is_empty());
            assert!(!inner.forks.votes_invalid.is_empty());
            assert!(!inner.forks.quorums_by_height.is_empty());

            assert!(!inner.valid.quorums_valid.is_empty());
            assert!(!inner.attestation_delay.time.is_empty());
        }

        // ------------------------------------------------------------------------
        // 5) Revert the chain and verify everything is cleared/reset.
        // ------------------------------------------------------------------------
        let reversion_info = stream::util::AttestationInfo {
            height: 50,
            digest: DIGEST_1.digest,
        };

        sx.note_attestation_chain_reversion(reversion_info);

        {
            let mut pool = rx.common.pool.lock();
            let inner = pool.expect_open();

            // Digest local reset
            assert_eq!(inner.digest_local, None);

            // Forks reset
            assert!(inner.forks.forks_by_digest.is_empty());
            assert!(inner.forks.forks_by_height.is_empty());
            assert!(inner.forks.forks_by_size.is_empty());
            assert_eq!(inner.forks.forks_best, None);

            assert!(inner.forks.pending_by_digest.is_empty());
            assert!(inner.forks.pending_by_prev_digest_tail.is_empty());
            assert!(inner.forks.pending_by_height.is_empty());

            assert!(inner.forks.votes.is_empty());
            assert!(inner.forks.votes_invalid.is_empty());
            assert!(inner.forks.quorums_by_height.is_empty());

            // Reversion should set the new finalized digest
            assert_eq!(inner.forks.last_finalized_digest, Some(DIGEST_1.digest));

            // Valid queue reset
            assert!(inner.valid.quorums_valid.is_empty());

            // Delay tracking reset
            assert!(inner.attestation_delay.time.is_empty());
        }
    }

    // -------------------------------- [ Height Validation Tests ] ------------------------------- //

    #[test]
    fn validate_height_rejects_misaligned() {
        let vq = ValidateQuorum::new(
            std::num::NonZeroUsize::new(2).unwrap(),
            std::num::NonZero::new(10u64).unwrap(),
            0,
            std::num::NonZero::new(500u64).unwrap(),
        );
        // Height 7 is not a multiple of interval 10
        assert!(!vq.validate_height(7, None));
        assert!(!vq.validate_height(15, None));
        // Aligned heights are accepted
        assert!(vq.validate_height(10, None));
        assert!(vq.validate_height(20, None));
        assert!(vq.validate_height(0, None));
    }

    #[test]
    fn validate_height_rejects_beyond_catchup_window() {
        let vq = ValidateQuorum::new(
            std::num::NonZeroUsize::new(2).unwrap(),
            std::num::NonZero::new(10u64).unwrap(),
            0,
            std::num::NonZero::new(5u64).unwrap(), // max_catchup = 5 → window = 5*10 = 50
        );
        // With no finalization, upper bound = start_height + 50 = 50
        assert!(vq.validate_height(50, None));
        assert!(!vq.validate_height(60, None));

        // With finalization at height 20, upper bound = 20 + 50 = 70
        assert!(vq.validate_height(70, Some(20)));
        assert!(!vq.validate_height(80, Some(20)));
    }

    #[test]
    fn validate_height_rejects_at_or_below_finalized() {
        let vq = ValidateQuorum::new(
            std::num::NonZeroUsize::new(2).unwrap(),
            std::num::NonZero::new(10u64).unwrap(),
            0,
            std::num::NonZero::new(500u64).unwrap(),
        );
        // Height at finalized is rejected
        assert!(!vq.validate_height(20, Some(20)));
        // Height below finalized is rejected
        assert!(!vq.validate_height(10, Some(20)));
        // First valid height above finalized
        assert!(vq.validate_height(30, Some(20)));
    }

    #[test]
    fn validate_height_accepts_start_height_when_no_finalization() {
        let vq = ValidateQuorum::new(
            std::num::NonZeroUsize::new(2).unwrap(),
            std::num::NonZero::new(10u64).unwrap(),
            100, // start_height = 100
            std::num::NonZero::new(500u64).unwrap(),
        );
        // Before any finalization, start_height is valid
        assert!(vq.validate_height(100, None));
        // Below start_height is rejected
        assert!(!vq.validate_height(90, None));
        // Above start_height and aligned is valid
        assert!(vq.validate_height(110, None));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_rejects_inadmissible_height(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 501, DIGEST_0)]
        attestation_far_future: AttestationVote,
        config: Config,
    ) {
        // Default config: start_height=1, interval=1, max_catchup=500,
        // last_finalized_height=Some(0). Height 501 exceeds window of 0 + 500*1 = 500
        let (sx, _rx) = attestation_pool(config);

        assert_matches::assert_matches!(
            sx.send(attestation_far_future.attestation.clone()),
            Some(Err(Error::InvalidHeight(ATTESTOR_VALID_0, 501, 0)))
        );
    }
}
