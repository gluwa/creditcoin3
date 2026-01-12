//! r strongly ordered data structure which efficiently keeps track of pending attestations.
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
//! instead returns an [`AttestationPermit`]. This permit _must_ be used by the [validation worker]
//! to mark the attestation as [`valid`] or [`invalid`] and remove it from the pool once it has
//! finished checking it, which is when the mutation occurs. This is done for several reasons:
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
//! # use attestor::worker::validation::pool::AttestorValidatePermissionless;
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
//! #   let config = attestor::worker::api::metrics::ConfigBuilder::new()
//! #       .with_name("test")
//! #       .with_address(cc_client::AccountId32([0; 32]))
//! #       .with_peer_id(libp2p::PeerId::random())
//! #       .with_chain_key(2u64)
//! #       .with_start_height(common::types::Height::MIN)
//! #       .with_attestation_latest_eth(common::types::Height::MIN)
//! #       .with_attestation_latest_cc3(common::types::Height::MIN)
//! #       .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
//! #       .build();
//! #   let metrics = std::sync::Arc::new(attestor::worker::api::metrics::Metrics::new(config));
//! #
//! // Initializes the attestation pool with some configuration
//! let (sx, mut rx) = attestation_pool(
//!     ConfigBuilder::new()
//!         .with_capacity(std::num::NonZeroUsize::new(100).unwrap())
//!         .with_quorum(std::num::NonZeroUsize::new(3).unwrap())
//!         .with_attestors(AttestorValidatePermissionless)
//!         .with_start_height(0u64)
//!         .with_attestation_interval(std::num::NonZeroU64::new(1).unwrap())
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

#[derive(Debug, attestor_macro::Builder)]
/// Attestation pool configuration options
pub struct Config {
    /// Maximum number of attestations which can be held in the pool before the pool begins
    /// evicting the highest attestations.
    #[allow(unused)]
    capacity: std::num::NonZeroUsize,

    /// Attestor validation policy, can be either [`AttestorValidatePermissionless`] or
    /// [`AttestorValidatePermissioned`].
    #[specify_later]
    attestors: Box<dyn ValidateAttestor>,

    /// Target [`Quorum`] size. Ie: the number of valid attestors which must submit the same
    /// attestation before it reaches quorum.
    #[specify_later]
    quorum: std::num::NonZeroUsize,

    /// Interval at which attestations are being produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    #[specify_later]
    attestation_interval: std::num::NonZero<common::types::Height>,

    /// Starting height at which attestation are produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    #[specify_later]
    start_height: common::types::Height,

    #[specify_later]
    digest_latest_cc3: Option<attestor_primitives::Digest>,

    #[specify_later]
    metrics: common::types::Metrics,
}

// ----------------------------------------- [ Types ] ----------------------------------------- //

type AttestationKey = (common::types::Height, attestor_primitives::Digest);

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
    tracing::info!(capacity = %config.capacity, "📮  with");
    tracing::info!(height = %config.start_height, "📮  with");
    tracing::info!(interval = %config.attestation_interval, "📮  with");
    tracing::info!(quorum = %config.quorum, "📮  with");
    tracing::info!(attestors = %config.attestors, "📮  with");

    let quorum = ValidateQuorum::new(
        config.quorum,
        config.start_height,
        config.attestation_interval,
        config.start_height,
    );

    let pool = AttestationPool::new(
        quorum,
        config.attestors,
        config.metrics,
        config.digest_latest_cc3,
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
            count_sender: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

impl Default for AttestationPoolCommon {
    fn default() -> Self {
        Self {
            pool: parking_lot::Mutex::new(AttestationPool::Closed),
            count_sender: std::sync::atomic::AtomicUsize::new(0),
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
    quorums: AttestationPoolQuorums,
    digest_local: Option<cc_client::H256>,

    validate_quorum: ValidateQuorum,
    validate_attestor: Box<dyn ValidateAttestor>,

    metrics: common::types::Metrics,
    attestation_delay: AttestationPoolDelays,

    wakers: std::collections::VecDeque<std::task::Waker>,
}

impl AttestationPool {
    fn new(
        validate_quorum: ValidateQuorum,
        validate_attestor: Box<dyn ValidateAttestor>,
        metrics: common::types::Metrics,
        prev_digest: Option<attestor_primitives::Digest>,
    ) -> Self {
        Self::Open(AttestationPoolInner {
            forks: AttestationPoolForks::new(prev_digest),
            quorums: AttestationPoolQuorums::new(),
            digest_local: None,

            validate_quorum,
            validate_attestor,

            metrics,
            attestation_delay: AttestationPoolDelays::new(),

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
    #[tracing::instrument(skip_all, fields(digest = %attestation.digest()))]
    fn push(&mut self, attestation: common::types::Attestation) -> Result<(), Error> {
        let height = attestation.header_number();

        tracing::debug!("Validating sender");
        self.validate_attestor.validate(&attestation)?;

        tracing::debug!(
            target_height = self.validate_quorum.target_height,
            "Validating height"
        );

        if attestation.header_number() < self.validate_quorum.target_height {
            return Err(Error::InvalidHeight(
                attestation.attestor.clone(),
                attestation.epoch,
                height,
                self.validate_quorum.target_height,
            ));
        }

        tracing::debug!("Checking for known invalids");

        let digest = attestation.digest();
        if self
            .forks
            .forks_invalid
            .get(&height)
            .is_some_and(|invalid| invalid.contains(&digest))
        {
            return Err(Error::InvalidDigest(
                attestation.attestor.clone(),
                attestation.epoch,
                height,
                digest,
            ));
        }

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

    fn peek(&mut self) -> Option<(Quorum, AttestationPermit)> {
        match self.forks.peek() {
            Some(fork) if self.validate_quorum.validate(&fork) => {
                let quorum = Quorum(fork.votes.clone());
                let height = fork.attestation.header_number();
                let key = (height, fork.attestation.digest());
                let permit = AttestationPermit(key);

                // Only update metrics the first time quorum is reached at that height
                if let Some(elapsed) = self.attestation_delay.pop(height) {
                    self.metrics
                        .update_attestation_delay_quorum(elapsed.as_secs_f64());
                }

                Some((quorum, permit))
            }
            _ => None,
        }
    }

    fn mark_valid(&mut self, permit: AttestationPermit) {
        let (height, digest) = permit.0;

        let removed = self
            .forks
            .forks_by_height
            .remove(&height)
            .expect("Missing mapping in forks_by_height");

        for digest in removed {
            self.forks.remove_by_digest(digest);
        }

        self.validate_quorum.target_height = util::next_multiple_of(
            self.validate_quorum.attestation_interval,
            self.validate_quorum.target_height,
        );
        self.digest_local = Some(cc_client::H256::from(digest.0));
    }

    fn mark_invalid(&mut self, permit: AttestationPermit) {
        let (height, digest) = permit.0;

        if let std::collections::btree_map::Entry::Occupied(mut entry) =
            self.forks.forks_by_height.entry(height)
        {
            let forks = entry.get_mut();
            assert!(forks.remove(&digest), "Missing mapping in forks_by_height");

            if forks.is_empty() {
                entry.remove();
            }

            self.forks.remove_by_digest(digest);

            self.forks
                .forks_invalid
                .entry(height)
                .or_default()
                .insert(digest);
        } else {
            panic!("Missing mapping in forks_by_height");
        }
    }

    fn mark_for_later(
        &mut self,
        permit: AttestationPermit,
        signed: common::types::AttestationSigned,
    ) {
        self.quorums.push(signed);
        self.mark_valid(permit);
    }
}

/// Holds and manages all attestation forks behind the execution chain finality. Keeps track of
/// contentious forks, past equivocations and known invalid votes. Attestation [`Quorum`]s which can
/// be validated ahead of finality are stored separately in an unbounded collection.
struct AttestationPoolForks {
    forks_by_size: std::collections::BTreeMap<
        usize,
        std::collections::HashMap<attestor_primitives::Digest, AttestationVote>,
    >,
    forks_by_digest: std::collections::HashMap<attestor_primitives::Digest, usize>,
    forks_by_height: std::collections::BTreeMap<
        common::types::Height,
        std::collections::HashSet<attestor_primitives::Digest>,
    >,
    forks_invalid: std::collections::BTreeMap<
        common::types::Height,
        std::collections::HashSet<attestor_primitives::Digest>,
    >,
    forks_best: Option<AttestationVote>,

    pending:
        std::collections::HashMap<attestor_primitives::Digest, Vec<common::types::Attestation>>,
    pending_count: usize,

    votes: std::collections::BTreeMap<
        common::types::Height,
        std::collections::HashMap<attestor_primitives::AttestorId, attestor_primitives::Digest>,
    >,

    prev_digest: Option<attestor_primitives::Digest>,
}

impl AttestationPoolForks {
    fn new(prev_digest: Option<attestor_primitives::Digest>) -> Self {
        Self {
            forks_by_size: Default::default(),
            forks_by_digest: Default::default(),
            forks_by_height: Default::default(),
            forks_invalid: Default::default(),
            forks_best: Default::default(),

            pending: Default::default(),
            pending_count: Default::default(),

            votes: Default::default(),

            prev_digest,
        }
    }

    #[tracing::instrument(skip_all, fields(height = attestation.header_number()))]
    fn push(&mut self, attestation: common::types::Attestation) -> Result<(), Error> {
        let epoch = attestation.epoch;
        let height = attestation.header_number();
        let digest = attestation.digest();
        let prev_digest = attestation.prev_digest();
        let attestor_id = attestation.attestor.clone();

        assert!(
            self.forks_invalid
                .get(&height)
                .is_none_or(|invalid| !invalid.contains(&digest)),
            "Trying to insert known invalid attestation"
        );

        match self
            .votes
            .entry(height)
            .or_default()
            .entry(attestor_id.clone())
        {
            std::collections::hash_map::Entry::Occupied(entry) => {
                let past_vote = entry.get();
                if past_vote != &digest {
                    return Err(Error::Equivocation(attestor_id, epoch, height));
                } else {
                    return Err(Error::AlreadyVoted(attestor_id, epoch, height));
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(digest);
            }
        }

        match (prev_digest, self.prev_digest) {
            // CASE 1] PREV_DIGEST MATCHES
            (Some(prev_digest_att), Some(prev_digest_pool))
                if prev_digest_att == prev_digest_pool => {}

            // CASE 2] NO PREV_DIGEST
            (None, None) => {}

            // CASE 3] UPDATE PREV_DIGEST
            (Some(prev_digest_att), None) => {
                self.prev_digest = Some(prev_digest_att);
            }

            // CASE 4] PENDING PREV_DIGET
            _ => {
                // NOTE: PROTOCOL
                //
                // It is possible to receive an empty prev_digest even after the chain has
                // finalized attestations due to network delay.
                if let Some(prev_digest) = prev_digest {
                    self.pending
                        .entry(prev_digest)
                        .or_default()
                        .push(attestation);
                    self.pending_count += 1;
                }

                return Ok(());
            }
        }

        let vote = AttestationVote::new(attestation);

        let size = self.forks_by_digest.entry(digest).or_default();
        let size_prev = *size;
        *size += 1;
        let size_new = *size;

        if let std::collections::btree_map::Entry::Occupied(mut entry) =
            self.forks_by_size.entry(size_prev)
        {
            let forks = entry.get_mut();
            assert!(
                forks.remove(&digest).is_some(),
                "Missing mapping in `forks_by_size`"
            );

            if forks.is_empty() {
                entry.remove();
            }
        }

        if self
            .forks_best
            .as_ref()
            .is_none_or(|best| best.votes.len() < size_new)
        {
            self.forks_best = Some(vote.clone());
        }

        self.forks_by_size
            .entry(size_new)
            .or_default()
            .insert(digest, vote);
        self.forks_by_height
            .entry(height)
            .or_default()
            .insert(digest);

        Ok(())
    }

    fn peek(&self) -> Option<AttestationVote> {
        self.forks_best.clone()
    }

    fn remove_by_digest(&mut self, digest: attestor_primitives::Digest) {
        let size = self
            .forks_by_digest
            .remove(&digest)
            .expect("Missing mapping in forks_by_digest");

        if let std::collections::btree_map::Entry::Occupied(mut entry) =
            self.forks_by_size.entry(size)
        {
            let votes = entry.get_mut();
            let removed = votes
                .remove(&digest)
                .expect("Missing mapping in forks_by_size");

            assert_eq!(
                removed.attestation.digest(),
                digest,
                "Invalid digest mapping in forks_by_size"
            );

            if votes.is_empty() {
                entry.remove();
            }
        } else {
            panic!("Missing mapping in forks_by_size");
        }

        self.forks_best = self.forks_by_size.last_key_value().map(|(_size, best)| {
            best.values()
                .next()
                .expect("Missing mapping in forks_by_size")
                .clone()
        });
    }
}

impl crate::events::EventAttestationFinalization for AttestationPoolForks {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization(
        &mut self,
        latest_attestation_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        let (prev_digest, height) = latest_attestation_cc3;
        self.prev_digest = Some(prev_digest);

        let mut removed = self.forks_by_height.split_off(&(height.saturating_add(1)));
        std::mem::swap(&mut self.forks_by_height, &mut removed);

        for digest in removed.into_values().flatten() {
            self.remove_by_digest(digest);
        }

        if let Some(pending) = self.pending.remove(&prev_digest) {
            for attestation in pending {
                if attestation.header_number() > height {
                    let digest = attestation.digest();

                    // WARNING: ERROR HANDLING
                    //
                    // We will need to bubble up this error once we implement peer scoring.
                    if let Err(err) = self.push(attestation) {
                        err.log_error(digest);
                    };
                }
            }
        }

        Ok(())
    }
}

impl crate::events::EventAttestationIntervalChange for AttestationPoolForks {
    type Error = std::convert::Infallible;

    async fn note_attestation_interval_change(
        &mut self,
        _interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        self.forks_by_size.clear();
        self.forks_by_digest.clear();
        self.forks_invalid.clear();
        self.forks_best = None;

        self.pending.clear();
        self.pending_count = 0;

        self.votes.clear();

        Ok(())
    }
}

struct AttestationPoolQuorums {
    quorums: std::collections::VecDeque<common::types::AttestationSigned>,
}

impl AttestationPoolQuorums {
    fn new() -> Self {
        Self {
            quorums: Default::default(),
        }
    }

    fn push(&mut self, signed: common::types::AttestationSigned) {
        let height = signed.attestation.header_number();

        assert!(
            self.quorums
                .front()
                .is_none_or(|signed| signed.attestation.header_number() < height),
            "Quorums must be sequential"
        );

        self.quorums.push_front(signed);
    }

    fn pop(
        &mut self,
    ) -> Option<(
        common::types::Height,
        attestor_primitives::Digest,
        cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
    )> {
        self.quorums
            .pop_back()
            .map(|att| (att.header_number(), att.digest(), att.into()))
    }
}

impl crate::events::EventAttestationFinalization for AttestationPoolQuorums {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization(
        &mut self,
        latest_attestation_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        let (_digest, height) = latest_attestation_cc3;

        while self
            .quorums
            .back()
            .is_some_and(|signed| signed.header_number() <= height)
        {
            self.quorums.pop_back();
        }

        Ok(())
    }
}

impl crate::events::EventAttestationIntervalChange for AttestationPoolQuorums {
    type Error = std::convert::Infallible;

    async fn note_attestation_interval_change(
        &mut self,
        _interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        self.quorums.clear();
        Ok(())
    }
}

struct AttestationPoolDelays {
    time: std::collections::BTreeMap<common::types::Height, std::time::Instant>,
}

impl AttestationPoolDelays {
    fn new() -> Self {
        Self {
            time: Default::default(),
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
}

impl crate::events::EventAttestationFinalization for AttestationPoolDelays {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization(
        &mut self,
        latest_attestation_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        let (_digest, height) = latest_attestation_cc3;
        self.time = self.time.split_off(&(height.saturating_add(1)));
        Ok(())
    }
}

impl crate::events::EventAttestationIntervalChange for AttestationPoolDelays {
    type Error = std::convert::Infallible;

    async fn note_attestation_interval_change(
        &mut self,
        _interval_new: std::num::NonZero<common::types::Height>,
        _attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        self.time.clear();
        Ok(())
    }
}

// ----------------------------------- [ Attestation Sender ] ---------------------------------- //

impl AttestationPoolSender {
    /// Sends an attestation to the attestation pool. Returns [`None`] if the pool has been
    /// [`closed`].
    ///
    /// [`closed`]: Self::close
    #[tracing::instrument(skip_all)]
    pub fn send(&self, attestation: common::types::Attestation) -> Option<Result<(), Error>> {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            tracing::debug!("Inserting attestation into the inner pool");
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

        inner.validate_quorum.target_quorum = quorum_new;

        if let Some(waker) = inner.wakers.pop_back() {
            tracing::debug!("Target sample size updated, waking receiver...");
            waker.wake();
        };
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl crate::events::EventAttestationFinalization for AttestationPoolSender {
    type Error = std::convert::Infallible;

    /// A new attestation has reached finality on the execution chain.
    ///
    /// Remove all attestations _up to and including_ that attestation height from the inner
    /// attestation pool.
    async fn note_attestation_finalization(
        &mut self,
        latest_attestation_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            let (_digest, _height) = latest_attestation_cc3;

            // Updating validation
            inner
                .validate_quorum
                .note_attestation_finalization(latest_attestation_cc3)
                .await
                .expect("Infallible");

            // Updating the inner pool
            inner
                .forks
                .note_attestation_finalization(latest_attestation_cc3)
                .await
                .expect("Infallible");

            // Remove past quorums
            inner
                .quorums
                .note_attestation_finalization(latest_attestation_cc3)
                .await
                .expect("Infallible");

            // Update metrics
            inner
                .attestation_delay
                .note_attestation_finalization(latest_attestation_cc3)
                .await
                .expect("Infallible");
        }

        Ok(())
    }
}

impl crate::events::EventAttestationIntervalChange for AttestationPoolSender {
    type Error = std::convert::Infallible;

    /// A new attestation interval has been set on-chain.
    //
    // Clear the attestation pool and update the target height and locally tracked attestation
    // interval.
    async fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            // Updating quorum
            inner
                .validate_quorum
                .note_attestation_interval_change(interval_new, attestation_latest_cc3)
                .await
                .expect("Infallible");

            // Updating the inner pool
            inner
                .forks
                .note_attestation_interval_change(interval_new, attestation_latest_cc3)
                .await
                .expect("Infallible");

            // Update metrics
            inner
                .attestation_delay
                .note_attestation_interval_change(interval_new, attestation_latest_cc3)
                .await
                .expect("Infallible");
        }

        Ok(())
    }
}

// Handling in response to execution chain events.
impl crate::events::EventAttestorsElected for AttestationPoolSender {
    type Error = std::convert::Infallible;

    async fn note_attestors_elected(
        &mut self,
        attestors: Vec<cc_client::AccountId32>,
    ) -> Result<(), Self::Error> {
        if let AttestationPool::Open(inner) = &mut *self.common.pool.lock() {
            tracing::warn!("🗂️ Updating the attestor set");

            inner.validate_attestor = Box::new(AttestorValidatePermissioned::new(
                std::collections::HashSet::from_iter(attestors.iter().map(|attestor| {
                    attestor_primitives::AttestorId::new(sp_core::crypto::AccountId32::new(
                        attestor.0,
                    ))
                })),
            ));
        }

        Ok(())
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
    pub fn mark_valid(&self, permit: AttestationPermit) {
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
    pub fn mark_invalid(&self, permit: AttestationPermit) {
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
        permit: AttestationPermit,
        signed: common::types::AttestationSigned,
    ) {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::debug!("Marking attestation for later removal");
                inner.mark_for_later(permit, signed);
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
    pub fn take_next_validated(
        &self,
    ) -> Option<(
        common::types::Height,
        attestor_primitives::Digest,
        cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
    )> {
        match &mut *self.common.pool.lock() {
            AttestationPool::Open(inner) => {
                tracing::debug!("Checking for next validated attestation");
                inner.quorums.pop()
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
    type Item = (Quorum, AttestationPermit, Option<cc_client::H256>);

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
            AttestationPool::Closed => {
                tracing::warn!("Tried to read attestation from pool after it has been closed!");
                std::task::Poll::Ready(None)
            }
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
            signers: hash_set![attestation.attestor.clone()],
            attestation,
        }
    }
}

impl std::cmp::PartialEq for AttestationVote {
    fn eq(&self, other: &Self) -> bool {
        self.attestation.header_number() == other.attestation.header_number()
            && self.attestation.digest() == other.attestation.digest()
    }
}

impl std::cmp::Eq for AttestationVote {}

impl std::hash::Hash for AttestationVote {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.attestation.header_number().hash(state);
        self.attestation.digest().hash(state);
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

    pub fn votes(self) -> Vec<common::types::Attestation> {
        self.0
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
pub struct AttestationPermit(AttestationKey);

// ------------------------------------ [ Quorum Validation ] ---------------------------------- //

/// Encapsulates quorum information to check if an attestation is ready for polling.
///
/// An attestation is ready for polling when enough attestors have voted for it and its height is
/// next in line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ValidateQuorum {
    target_quorum: std::num::NonZeroUsize,
    target_height: common::types::Height,
    attestation_interval: std::num::NonZero<common::types::Height>,
    start_height: common::types::Height,
}

impl ValidateQuorum {
    pub const fn new(
        target_quorum: std::num::NonZeroUsize,
        target_height: common::types::Height,
        attestation_interval: std::num::NonZero<common::types::Height>,
        start_height: common::types::Height,
    ) -> Self {
        Self {
            target_quorum,
            target_height,
            attestation_interval,
            start_height,
        }
    }

    #[tracing::instrument(skip_all, fields(target_height = %self.target_height, target_quorum = %self.target_quorum))]
    fn validate(&self, attestation: &AttestationVote) -> bool {
        tracing::debug!(
            height = attestation.attestation.header_number(),
            quorum = attestation.signers.len(),
            "Validating attestation"
        );

        attestation.attestation.header_number() == self.target_height
            && attestation.signers.len() >= self.target_quorum.into()
    }
}

impl crate::events::EventAttestationFinalization for ValidateQuorum {
    type Error = std::convert::Infallible;

    async fn note_attestation_finalization(
        &mut self,
        latest_attestation_cc3: (attestor_primitives::Digest, common::types::Height),
    ) -> Result<(), Self::Error> {
        let (_digest, height) = latest_attestation_cc3;
        let height_new = util::next_multiple_of(self.attestation_interval, height);

        if self.target_height < height_new {
            self.target_height = height_new;
        }

        Ok(())
    }
}

impl crate::events::EventAttestationIntervalChange for ValidateQuorum {
    type Error = std::convert::Infallible;

    async fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<common::types::Height>,
        attestation_latest_cc3: Option<common::types::Height>,
    ) -> Result<(), Self::Error> {
        let target_height_new = if let Some(attestation_latest_cc3) = attestation_latest_cc3 {
            util::next_multiple_of(interval_new, attestation_latest_cc3)
        } else {
            self.start_height
        };

        self.attestation_interval = interval_new;
        self.target_height = target_height_new;

        Ok(())
    }
}

// ----------------------------------- [ Attestor Validation ] --------------------------------- //

/// Common trait used to determine if an attestor can submit attestations.
pub trait ValidateAttestor: Send + Sync + std::fmt::Debug + std::fmt::Display + 'static {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), Error>;
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

impl ValidateAttestor for AttestorValidatePermissioned {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), Error> {
        if !self.attestor_set.contains(&attestation.attestor) {
            return Err(Error::Unauthorized(
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

impl ValidateAttestor for AttestorValidatePermissionless {
    fn validate(&self, _attestation: &common::types::Attestation) -> Result<(), Error> {
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

impl ValidateAttestor for AttestorValidateDeny {
    fn validate(&self, attestation: &common::types::Attestation) -> Result<(), Error> {
        Err(Error::Unauthorized(
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

impl std::fmt::Display for ValidateQuorum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ vote_count: {} }}", self.target_quorum)
    }
}

impl std::fmt::Display for AttestationPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{ height: {}, digest: {} }}", self.0 .0, self.0 .1)
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
    ) -> AttestationVote {
        let mut iter = attestors.into_iter();

        let attestation = common::types::Attestation {
            attestation_data: attestor_primitives::AttestationData {
                header_number,
                prev_digest: Some(prev_digest),
                ..Default::default()
            },
            attestor: iter.next().unwrap(),
            signature: Default::default(),
            signature_bls: attestor_primitives::bls::WrapEncode(
                bls_signatures::PrivateKey::new(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
                    .sign(b"0xdeadbeef"),
            ),
            continuity_proof: Default::default(),
            epoch,
        };

        iter.fold(
            AttestationVote {
                votes: vec![attestation.clone()],
                signers: hash_set![attestation.attestor.clone()],
                attestation,
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
        #[default(0)] _epoch: common::types::Epoch,
        #[default(0)] _header_number: common::types::Height,
        #[default(DIGEST_0)] _prev_digest: attestor_primitives::Digest,
        #[with(_attestors.clone(), _epoch, _header_number, _prev_digest)]
        attestation: AttestationVote,
    ) -> Quorum {
        Quorum(attestation.votes)
    }

    #[rstest::fixture]
    pub fn quorum_validate(#[default(2)] vote_count: usize) -> ValidateQuorum {
        ValidateQuorum {
            target_quorum: vote_count.try_into().unwrap(),
            target_height: 0,
            start_height: 0,
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
    pub fn metrics() -> common::types::Metrics {
        let config = crate::worker::api::metrics::ConfigBuilder::new()
            .with_name("test")
            .with_address(cc_client::AccountId32([0; 32]))
            .with_peer_id(libp2p::PeerId::random())
            .with_chain_key(2u64)
            .with_start_height(common::types::Height::MIN)
            .with_attestation_latest_eth(common::types::Height::MIN)
            .with_attestation_latest_cc3(common::types::Height::MIN)
            .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
            .build();
        std::sync::Arc::new(crate::worker::api::metrics::Metrics::new(config))
    }

    #[rstest::fixture]
    pub fn config(
        quorum_validate: ValidateQuorum,
        #[default(100)] capacity: usize,
        permissioned: AttestorValidatePermissioned,
        metrics: common::types::Metrics,
    ) -> Config {
        ConfigBuilder::new()
            .with_capacity(std::num::NonZeroUsize::new(capacity).unwrap())
            .with_attestors(permissioned)
            .with_quorum(quorum_validate.target_quorum)
            .with_attestation_interval(std::num::NonZero::<common::types::Height>::MIN)
            .with_start_height(common::types::Height::MIN)
            .with_metrics(metrics)
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
        attestation: AttestationVote,
    ) -> AttestationPermit {
        AttestationPermit((
            attestation.attestation.header_number(),
            attestation.attestation.digest(),
        ))
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
    async fn attestation_pool_sanity_basic(
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
        permit: AttestationPermit,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.votes[0].clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).unwrap().is_ok());

        let actual = rx.next().await;
        let expected = Some((quorum, permit, None));

        assert_eq!(actual, expected,);
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_valid(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 0, 0, DIGEST_0)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 0, 0, DIGEST_0)]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_2], 0, 0, DIGEST_1)]
        attestation_2: AttestationVote,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0, ATTESTOR_VALID_1], 0, 0, DIGEST_0)]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);

        assert!(sx.send(attestation_0.votes[0].clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).unwrap().is_ok());
        assert!(sx.send(attestation_2.votes[0].clone()).unwrap().is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);

        rx.mark_valid(permit);

        let mut pool = rx.common.pool.lock();
        let inner = pool.expect_open();

        assert!(!inner.forks.forks.contains_key(&0));
        assert_eq!(
            inner.digest_local,
            Some(cc_client::H256(attestation_1.votes[0].digest().0))
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

        assert_eq!(inner.forks.forks.len(), 0);
        assert!(inner
            .forks
            .invalid
            .get(&0)
            .unwrap()
            .contains(&attestation_0.attestation.digest()));
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

        assert!(sx.send(attestation_0.votes[0].clone()).unwrap().is_ok());
        assert!(sx.send(attestation_1.votes[0].clone()).unwrap().is_ok());

        let (quorum_actual, permit, _digest_local) = rx.next().await.unwrap();

        assert_eq!(quorum_actual, quorum_expected);
        rx.mark_for_later(permit, attestation_signed.clone());

        // Such types, much wow... -fuck subxt and the incompatible dependencies which make using
        // our own types an even more royal pain $$%%^#$#
        let attestation_expected: cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        > = attestation_signed.clone().into();

        assert_matches::assert_matches!(rx.take_next_validated(), Some((height, digest, attestation)) => {
            assert_eq!(height, attestation_0.attestation.header_number());
            assert_eq!(digest, attestation_0.attestation.digest());
            // Other types in this don't implement PartialEq and Eq...
            assert_eq!(attestation.attestors, attestation_expected.attestors);
        });

        assert_eq!(
            sx.common.pool.lock().expect_open().digest_local,
            Some(cc_client::H256(attestation_signed.digest().0))
        );
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
            sx.send(attestation.votes[0].clone()),
            Some(Err(Error::Unauthorized(ATTESTOR_INVALID, 0, 0)))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_async_wake_receiver(
        _logs: (),
        #[with([ATTESTOR_VALID_0])] attestation: AttestationVote,
        #[with([ATTESTOR_VALID_0])] permit: AttestationPermit,
        #[with([ATTESTOR_VALID_0])] quorum: Quorum,
        #[from(quorum_validate)]
        #[with(1)]
        _quorum_validate: ValidateQuorum,
        #[with(_quorum_validate.clone())] config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let (sx, mut rx) = attestation_pool(config);
        let mut fut = tokio_test::task::spawn(rx.next());

        tokio_test::assert_pending!(fut.poll());
        assert!(sx.send(attestation.votes[0].clone()).unwrap().is_ok());
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
        #[with([ATTESTOR_VALID_1])] attestation: AttestationVote,
        config: Config,
    ) {
        let (sx, rx) = attestation_pool(config);
        rx.close();
        assert_matches::assert_matches!(sx.send(attestation.votes[0].clone()), None);
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
        assert!(sx.send(attestation.votes[0].clone()).unwrap().is_ok());

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
        quorum_validate: ValidateQuorum,
    ) {
        assert!(quorum_validate.validate(&attestation_0));
        assert!(!quorum_validate.validate(&attestation_1));
    }

    #[rstest::rstest]
    fn validator_parameters_validate_permissioned(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationVote,
        permissioned: AttestorValidatePermissioned,
    ) {
        assert!(permissioned.validate(&attestation_0.votes[0]).is_ok());
        assert_matches::assert_matches!(
            permissioned.validate(&attestation_2.votes[0]),
            Err(Error::Unauthorized(ATTESTOR_INVALID, 0, 0))
        );
    }

    #[rstest::rstest]
    fn validator_parameters_validate_permissionless(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationVote,
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
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_INVALID])]
        attestation_2: AttestationVote,
        deny: AttestorValidateDeny,
    ) {
        assert_matches::assert_matches!(
            deny.validate(&attestation_0.votes[0]),
            Err(Error::Unauthorized(ATTESTOR_VALID_0, 0, 0))
        );
        assert_matches::assert_matches!(
            deny.validate(&attestation_2.votes[0]),
            Err(Error::Unauthorized(ATTESTOR_INVALID, 0, 0))
        );
    }
}
