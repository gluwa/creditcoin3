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
//! #       .with_start_height(attestor_primitives::Height::MIN)
//! #       .with_genesis(attestor_primitives::Height::MIN)
//! #       .with_attestation_latest_eth(attestor_primitives::Height::MIN)
//! #       .with_attestation_interval(std::num::NonZero::<attestor_primitives::Height>::MIN)
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
mod metrics;

pub use error::*;
pub use metrics::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(builder::Builder)]
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
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,

    /// Starting height at which attestation are produced. This value is fetched from on-chain
    /// storage unless it is overridden in [attestation config].
    ///
    /// [attestation config]: crate::attestation
    start_height: attestor_primitives::Height,

    /// Maximum number of attestation intervals an attestor can catch up from the latest finalized
    /// attestation. Votes beyond this window are rejected to prevent pool-filling DoS attacks.
    max_catchup: std::num::NonZero<attestor_primitives::Height>,

    /// Latest execution chain digest, used to validate the tail prev digest of new attestations.
    start_attestation: Option<stream::util::AttestationInfo>,

    metrics: Box<dyn MetricsAttestationPool>,
}

// ------------------------------------ [ Attestation Pool ] ----------------------------------- //

/// Concrete implementation of the attestation pool, holding all of the implementation logic.
struct AttestationPool {
    forks: AttestationPoolForks,
    valid: AttestationPoolValid,
    digest_local: Option<cc_client::H256>,

    validate_attestor: ValidateAttestor,

    metrics: Box<dyn MetricsAttestationPool>,
    attestation_delay: AttestationPoolDelays,

    wakers: std::collections::VecDeque<std::task::Waker>,
}

impl AttestationPool {
    fn new(config: Config) -> Self {
        let validate_quorum = ValidateQuorum::new(
            config.quorum,
            config.attestation_interval,
            config.start_height,
            config.max_catchup,
        );

        let validate_attestor = ValidateAttestor::new(config.attestors);

        Self {
            forks: AttestationPoolForks::new(
                config.start_attestation.map(|info| info.digest),
                config.start_attestation.map(|info| info.height),
                validate_quorum,
            ),
            valid: AttestationPoolValid::new(),
            digest_local: None,

            validate_attestor,

            attestation_delay: AttestationPoolDelays::new(),
            metrics: config.metrics,

            wakers: std::collections::VecDeque::new(),
        }
    }
}

impl AttestationPool {
    pub fn send(&mut self, attestation: common::types::Attestation) -> Result<(), Error> {
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

    pub fn mark_valid(&mut self, Permit(info): Permit) {
        self.forks.split_off(info.height);
        self.forks.forks_best = self.forks.find_best();
        self.digest_local = Some(cc_client::H256::from(info.digest.digest.0));
    }

    pub fn mark_invalid(&mut self, Permit(info): Permit) {
        self.forks.pop(info.into());
    }

    pub fn mark_for_later(
        &mut self,
        permit: Permit,
        signed: common::types::AttestationSigned,
        votes: Vec<common::types::Attestation>,
    ) {
        self.valid.push(signed, votes);
        self.mark_valid(permit);
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
        &mut self,
    ) -> Option<(
        attestor_primitives::Height,
        attestor_primitives::Digest,
        cc_client::cc3::runtime_types::attestor_primitives::SignedAttestation<
            cc_client::H256,
            cc_client::AccountId32,
        >,
        Vec<common::types::Attestation>,
    )> {
        tracing::debug!("Checking for next validated attestation");
        self.valid.pop()
    }

    fn peek(&mut self) -> Option<(Quorum, Permit)> {
        self.forks.peek().map(|fork| {
            let quorum = Quorum(fork.votes.clone());
            let height = fork.attestation.header_number();
            let digest = fork.attestation.digest();
            let header_hash = fork.attestation.attestation_data.header_hash;

            let digest_continuity = fork.attestation.continuity_proof.compute_continuity_digest(
                fork.attestation
                    .continuity_proof
                    .start_block_number(fork.attestation.header_number()),
            );

            let permit = Permit(CompoundInfo {
                height,
                digest: CompoundDigest {
                    digest,
                    digest_continuity,
                    header_hash,
                },
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
}

impl futures::Stream for AttestationPool {
    type Item = (Quorum, Permit, Option<cc_client::H256>);

    /// This future is cancellation-safe, as it does not perform any mutations on the inner pool.
    #[tracing::instrument(skip_all)]
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.peek() {
            Some((quorum, permit)) => {
                tracing::debug!(height = quorum.header_number(), "Found a quorum");
                std::task::Poll::Ready(Some((quorum, permit, self.digest_local)))
            }
            None => {
                tracing::debug!("No quorum found, waiting for new attestations...");
                self.wakers.push_front(cx.waker().clone());
                std::task::Poll::Pending
            }
        }
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl AttestationPool {
    #[tracing::instrument(skip_all, fields(target_sample_size), level = "debug")]
    pub fn note_target_sample_size_change(&mut self, target_sample_size: u32) {
        let threshold = attestor_primitives::calculate_threshold(target_sample_size) as usize;
        let quorum_new = std::num::NonZeroUsize::new(threshold);

        let Some(quorum_new) = quorum_new else {
            return;
        };

        self.forks.note_target_sample_size_change(quorum_new);

        if let Some(waker) = self.wakers.pop_back() {
            tracing::debug!("Target sample size updated, waking receiver...");
            waker.wake();
        };
    }

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
        // Remove past quorums
        self.valid.note_attestation_finalization(info);

        // Update metrics
        self.attestation_delay
            .note_attestation_finalization(info, &self.metrics);

        // Updating the inner pool
        self.forks.note_attestation_finalization(info)?;

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
        interval_new: std::num::NonZero<attestor_primitives::Height>,
    ) {
        self.digest_local = None;

        // Updating the inner pool
        self.forks.note_attestation_interval_change(interval_new);

        // Updating quorums
        self.valid.note_attestation_interval_change();

        // Update metrics
        self.attestation_delay.note_attestation_interval_change();
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
        tracing::warn!("🗂️ Updating the attestor set");
        self.validate_attestor = ValidateAttestor::new(attestors);
    }

    /// An attestation chain reversion has been detected.
    /// We need to clear the structures `forks`, `valid`, and `attestation_delay`
    #[tracing::instrument(
        skip_all,
        fields(digest = ?info.digest, height = info.height),
        level = "debug"
    )]
    pub fn note_attestation_chain_reversion(&mut self, info: stream::util::AttestationInfo) {
        // Clear digest local, as it no longer tracks a valid new attestation
        self.digest_local = None;
        // Updating the inner pool
        self.forks.note_attestation_chain_reversion(info);

        // Remove past quorums
        self.valid.note_attestation_chain_reversion();

        // Update metrics
        self.attestation_delay.note_attestation_chain_reversion();
    }
}

// ------------------------------------ [ Attestation Forks ] ----------------------------------- //

/// Orders attestations by height.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyHeight {
    height: attestor_primitives::Height,
    size: usize,
    digest: CompoundDigest,
}

/// Orders attestations by quorum size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeySize {
    size: usize,
    height: attestor_primitives::Height,
    digest: CompoundDigest,
}

/// Orders attestor votes by height.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyVote {
    height: attestor_primitives::Height,
    attestor: attestor_primitives::AttestorId,
}

/// Orders votes by their digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyDigest {
    height: attestor_primitives::Height,
    digest: CompoundDigest,
}

/// Orders pending votes by their digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyHeightPending {
    height: attestor_primitives::Height,
    digest: CompoundDigest,
    prev_digest_tail: PrevDigestTail,
}

/// Orders pending votes by their prev tail digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyTailPending {
    prev_digest_tail: PrevDigestTail,
    height: attestor_primitives::Height,
    digest: CompoundDigest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct KeyDigestPending {
    height: attestor_primitives::Height,
    digest: CompoundDigest,
    prev_digest_tail: PrevDigestTail,
}

/// Attestation [digest computation] does not account for all fields in an [`Attestation`].
/// Namely, the attestation `header_hash` is absent from digest computation yet is still used for
/// [attestation data serialization], **which is what attestors sign on**. The attestation
/// continuity proof is absent too, even though it is submitted to the runtime alongside the signed
/// data. This means the attestation digest alone is not a guarantee of uniqueness, and must be
/// paired with the header hash and continuity proof digest to avoid collisions.
///
/// [digest computation]: attestor_primitives::Attestation::digest
/// [`Attestation`]:  common::types::Attestation
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
    height: attestor_primitives::Height,
    digest: CompoundDigest,
}

impl From<CompoundInfo> for CompoundDigest {
    fn from(info: CompoundInfo) -> Self {
        info.digest
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
    last_finalized_height: Option<attestor_primitives::Height>,
    validate_quorum: ValidateQuorum,
}

impl AttestationPoolForks {
    fn new(
        last_finalized_digest: Option<attestor_primitives::Digest>,
        last_finalized_height: Option<attestor_primitives::Height>,
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

        let digest_continuity = attestation
            .continuity_proof
            .compute_continuity_digest(attestation.continuity_proof.start_block_number(height));

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
            self.votes_invalid.insert(key_digest),
            "Duplicate mapping in votes_invalid: {key_digest:#?}"
        );

        // WARNING: RACE CONDITION
        //
        // The entry for this digest in `quorums_by_height` may have been removed following a
        // target sample size update since quorum was last observed.
        let _ = self.quorums_by_height.remove(&key_height);

        for attestor in vote.signers {
            let key_vote = KeyVote { height, attestor };
            self.votes
                .remove(&key_vote)
                .expect("Missing mapping in votes_valid");
        }

        self.forks_best = self.find_best();
    }

    fn split_off(&mut self, height: attestor_primitives::Height) {
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
            .last()
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
            height: attestor_primitives::Height::MAX,
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
        interval_new: std::num::NonZero<attestor_primitives::Height>,
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

    /// Updating the target sample size is a sparse operation which may invalidate past quorums. As
    /// a result, we discard all past quorums and recompute the set of quorums under the new target
    /// sample size.
    ///
    /// While this has `O(n)` complexity, in practice this is a rare enough event that the cost of
    /// recomputing quorums is negligible once amortized.
    fn note_target_sample_size_change(&mut self, quorum_new: std::num::NonZeroUsize) {
        self.quorums_by_height.clear();

        let key_start = KeySize {
            size: quorum_new.into(),
            height: 0,
            digest: CompoundDigest::min(),
        };
        let index = (
            std::ops::Bound::Included(key_start),
            std::ops::Bound::Unbounded,
        );

        for KeySize {
            size,
            height,
            digest,
        } in self.forks_by_size.range(index).copied()
        {
            let key_height = KeyHeight {
                height,
                size,
                digest,
            };
            self.quorums_by_height.insert(key_height);
        }

        self.validate_quorum.target_quorum = quorum_new;
        self.forks_best = self.find_best();
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
        attestor_primitives::Height,
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
        attestor_primitives::Height,
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

#[derive(Default)]
struct AttestationPoolDelays {
    time: std::collections::BTreeMap<attestor_primitives::Height, std::time::Instant>,
}

impl AttestationPoolDelays {
    fn new() -> Self {
        Self::default()
    }

    fn push(&mut self, height: attestor_primitives::Height) {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.time.entry(height) {
            entry.insert(std::time::Instant::now());
        }
    }

    fn pop(&mut self, height: attestor_primitives::Height) -> Option<std::time::Duration> {
        self.time.remove(&height).map(|then| then.elapsed())
    }

    fn note_attestation_finalization(
        &mut self,
        info: stream::util::AttestationInfo,
        metrics: &dyn MetricsAttestationPool,
    ) {
        tracing::debug!("Updating quorum delays");

        let mut removed = self.time.split_off(&(info.height.saturating_add(1)));
        std::mem::swap(&mut self.time, &mut removed);

        if let Some(then) = removed.get(&info.height) {
            metrics.update_attestation_delay_finalization(then.elapsed());
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

    #[cfg(test)]
    fn compound_digest(&self) -> CompoundDigest {
        let digest = self.attestation.digest();
        let header_hash = self.attestation.attestation_data.header_hash;

        let digest_continuity = self.attestation.continuity_proof.compute_continuity_digest(
            self.attestation
                .continuity_proof
                .start_block_number(self.attestation.header_number()),
        );

        CompoundDigest {
            digest,
            digest_continuity,
            header_hash,
        }
    }

    #[cfg(test)]
    fn compound_info(&self) -> CompoundInfo {
        CompoundInfo {
            height: self.attestation.header_number(),
            digest: self.compound_digest(),
        }
    }
}

impl std::cmp::PartialEq for AttestationVote {
    fn eq(&self, other: &Self) -> bool {
        let matching_digest = || self.attestation.digest() == other.attestation.digest();

        let matching_header = || {
            self.attestation.attestation_data.header_hash
                == other.attestation.attestation_data.header_hash
        };

        let matching_continuity = || {
            let start_self = self
                .attestation
                .continuity_proof
                .start_block_number(self.attestation.header_number());
            let continuity_digest_self = self
                .attestation
                .continuity_proof
                .compute_continuity_digest(start_self);

            let start_other = other
                .attestation
                .continuity_proof
                .start_block_number(other.attestation.header_number());
            let continuity_digest_other = other
                .attestation
                .continuity_proof
                .compute_continuity_digest(start_other);

            continuity_digest_self == continuity_digest_other
        };

        // Attestation header number is implied in the digest computation and so does not need to
        // be checked manually as changing it would result in a different digest. The header hash
        // and continuity proof are NOT part of digest computation and needs to be checked manually
        matching_digest() && matching_header() && matching_continuity()
    }
}

impl std::cmp::Eq for AttestationVote {}

impl std::hash::Hash for AttestationVote {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.attestation.digest().hash(state);
        self.attestation.attestation_data.header_hash.hash(state);

        let start = self
            .attestation
            .continuity_proof
            .start_block_number(self.attestation.header_number());
        let digest_continuity = self
            .attestation
            .continuity_proof
            .compute_continuity_digest(start);

        digest_continuity.hash(state);
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

    pub fn header_number(&self) -> attestor_primitives::Height {
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
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    start_height: attestor_primitives::Height,
    max_catchup: std::num::NonZero<attestor_primitives::Height>,
}

impl ValidateQuorum {
    pub const fn new(
        target_quorum: std::num::NonZeroUsize,
        attestation_interval: std::num::NonZero<attestor_primitives::Height>,
        start_height: attestor_primitives::Height,
        max_catchup: std::num::NonZero<attestor_primitives::Height>,
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

    /// Validates that a vote height is admissible: above the latest finalized height, and within
    /// the maximum catch-up window.
    fn validate_height(
        &self,
        height: attestor_primitives::Height,
        last_finalized_height: Option<attestor_primitives::Height>,
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

        above_finalized && height >= self.start_height && height <= upper_bound
    }

    fn note_attestation_interval_change(
        &mut self,
        interval_new: std::num::NonZero<attestor_primitives::Height>,
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
            self.0.height, self.0.digest.digest
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

    pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
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
        #[default(2)] header_number: attestor_primitives::Height,
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
        #[default(2)] _header_number: attestor_primitives::Height,
        #[default(DIGEST_0)] _prev_digest: CompoundDigest,
        #[default(DIGEST_0)] _header_hash: CompoundDigest,
        #[with(_attestors.clone(), _header_number, _prev_digest, _header_hash)]
        attestation: AttestationVote,
    ) -> Quorum {
        Quorum(attestation.votes)
    }

    #[rstest::fixture]
    pub fn validate_quorum(
        #[default(2)] vote_count: usize,
        #[default(1)] attestation_interval: attestor_primitives::Height,
        #[default(1)] start_height: attestor_primitives::Height,
        #[default(common::constants::MAX_CATCHUP.get())] max_catchup: attestor_primitives::Height,
    ) -> ValidateQuorum {
        ValidateQuorum {
            target_quorum: vote_count.try_into().unwrap(),
            attestation_interval: attestation_interval.try_into().unwrap(),
            start_height,
            max_catchup: max_catchup.try_into().unwrap(),
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
    pub fn metrics() -> Box<dyn MetricsAttestationPool> {
        struct Metrics;

        impl MetricsAttestationPool for Metrics {
            fn update_attestation_delay_quorum(&self, _delay: std::time::Duration) {}

            fn update_attestation_delay_finalization(&self, _delay: std::time::Duration) {}
        }

        Box::new(Metrics)
    }

    #[rstest::fixture]
    pub fn config(
        validate_quorum: ValidateQuorum,
        attestors: Vec<cc_client::AccountId32>,
        metrics: Box<dyn MetricsAttestationPool>,
    ) -> Config {
        ConfigBuilder::new()
            .with_attestors(attestors)
            .with_quorum(validate_quorum.target_quorum)
            .with_attestation_interval(std::num::NonZero::<attestor_primitives::Height>::MIN)
            .with_start_attestation(Some(stream::util::AttestationInfo {
                digest: DIGEST_0.digest,
                height: attestor_primitives::Height::MIN,
            }))
            .with_start_height(1u64)
            .with_max_catchup(common::constants::MAX_CATCHUP)
            .with_metrics(metrics)
            .build()
    }

    #[rstest::fixture]
    pub fn permit(
        #[default([ATTESTOR_VALID_0])] _attestors: impl IntoIterator<Item = attestor_primitives::AttestorId>
            + Clone,
        #[default(2)] _header_number: attestor_primitives::Height,
        #[default(DIGEST_0)] _prev_digest: CompoundDigest,
        #[default(DIGEST_0)] _header_hash: CompoundDigest,
        #[with(_attestors.clone(), _header_number, _prev_digest, _header_hash)]
        attestation: AttestationVote,
    ) -> Permit {
        Permit(attestation.compound_info())
    }
}

// -------------------------------------- [ Sanity Checks ] ------------------------------------ //

#[cfg(test)]
mod test {
    use common::fixtures::*;

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

        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());

        let (quorum_actual, permit, _digest_local) = pool.next().await.unwrap();

        pretty_assertions::assert_eq!(quorum_actual, quorum_expected);

        pool.mark_valid(permit);

        assert!(!pool.forks.forks_by_height.contains(&KeyHeight {
            height: 1,
            size: 2,
            digest: DIGEST_0
        }));
        pretty_assertions::assert_eq!(
            pool.digest_local,
            Some(cc_client::H256(attestation_1.attestation.digest().0))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_invalid_simple(
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

        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());

        let (quorum_actual, permit, _digest_local) = pool.next().await.unwrap();

        pretty_assertions::assert_eq!(quorum_actual, quorum_expected);
        pool.mark_invalid(permit);

        assert!(pool.forks.votes_invalid.contains(&KeyDigest {
            height: attestation_0.attestation.header_number(),
            digest: attestation_0.compound_digest()
        }));

        assert!(pool.forks.votes_invalid.contains(&KeyDigest {
            height: attestation_1.attestation.header_number(),
            digest: attestation_1.compound_digest()
        }));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_mark_invalid_no_panic(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0])]
        attestation_0: AttestationVote,
        #[from(quorum)]
        #[with([ATTESTOR_VALID_0])]
        quorum_expected: Quorum,
        config: Config,
    ) {
        use futures::stream::StreamExt as _;

        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());

        // A vote which is note yet in `quorums_by_height` reaches quorum
        pool.note_target_sample_size_change(1);

        let (quorum_actual, permit, _digest_local) = pool.next().await.unwrap();

        // `mark_invalid` removes quorum under new validation rules. `quorums_by_height` must have
        // been updated by now or this will panic!
        pretty_assertions::assert_eq!(quorum_actual, quorum_expected);
        pool.mark_invalid(permit);

        assert!(pool.forks.votes_invalid.contains(&KeyDigest {
            height: attestation_0.attestation.header_number(),
            digest: attestation_0.compound_digest()
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

        let mut pool = AttestationPool::new(config);

        assert_matches::assert_matches!(pool.take_next_validated(), None);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());

        let (quorum_actual, permit, _digest_local) = pool.next().await.unwrap();

        pretty_assertions::assert_eq!(quorum_actual, quorum_expected);
        pool.mark_for_later(
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

        assert_matches::assert_matches!(pool.take_next_validated(), Some((height, digest, attestation, votes)) => {
            pretty_assertions::assert_eq!(height, attestation_0.attestation.header_number());
            pretty_assertions::assert_eq!(digest, attestation_0.attestation.digest());
            // Other types in this don't implement PartialEq and Eq...
            pretty_assertions::assert_eq!(attestation.attestors, attestation_expected.attestors);
            pretty_assertions::assert_eq!(votes,
                vec![
                    attestation_0.attestation,
                    attestation_1.attestation,
                ],
            );
        });

        pretty_assertions::assert_eq!(
            pool.digest_local,
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
        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_pending.attestation.clone()).is_ok());

        pretty_assertions::assert_eq!(pool.forks.pending_by_digest.len(), 1);
        pretty_assertions::assert_eq!(pool.forks.pending_by_prev_digest_tail.len(), 1);
        pretty_assertions::assert_eq!(pool.forks.pending_by_height.len(), 1);

        assert!(pool
            .forks
            .pending_by_prev_digest_tail
            .contains(&KeyTailPending {
                prev_digest_tail: PrevDigestTail(DIGEST_1.digest),
                height: 2,
                digest: attestation_pending.compound_digest(),
            }));

        pool.note_attestation_finalization(stream::util::AttestationInfo {
            digest: DIGEST_1.digest,
            height: 1,
        })
        .unwrap();

        let vote = AttestationVote::new(attestation_pending.attestation.clone());
        pretty_assertions::assert_eq!(pool.forks.forks_best.clone().unwrap(), vote);
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
        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());

        pretty_assertions::assert_eq!(pool.forks.votes.len(), 2);
        pretty_assertions::assert_eq!(pool.forks.forks_by_digest.len(), 2);
        pretty_assertions::assert_eq!(pool.forks.forks_by_size.len(), 2);

        pretty_assertions::assert_eq!(
            pool.forks
                .forks_by_digest
                .get(&attestation_0.compound_digest())
                .unwrap(),
            &attestation_0
        );

        pretty_assertions::assert_eq!(
            pool.forks
                .forks_by_digest
                .get(&attestation_1.compound_digest())
                .unwrap(),
            &attestation_1
        );

        assert!(pool.forks.forks_by_size.contains(&KeySize {
            size: 1,
            height: 2,
            digest: attestation_0.compound_digest()
        }));

        assert!(pool.forks.forks_by_size.contains(&KeySize {
            size: 1,
            height: 2,
            digest: attestation_1.compound_digest()
        }));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_deduplicate_continuity_proof(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 3)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 3)]
        mut attestation_1: AttestationVote,
        config: Config,
    ) {
        let mut pool = AttestationPool::new(config);

        attestation_1.attestation.continuity_proof =
            attestor_primitives::block::ContinuityProof::new(
                attestation_1.attestation.prev_digest().unwrap(),
                vec![
                    attestor_primitives::Digest::default(),
                    attestor_primitives::Digest::default(),
                ],
            );

        assert_ne!(
            attestation_0.attestation.continuity_proof, attestation_1.attestation.continuity_proof,
            "Attestation continuity proofs must not match for this test"
        );

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());

        pretty_assertions::assert_eq!(pool.forks.votes.len(), 2);
        pretty_assertions::assert_eq!(pool.forks.forks_by_digest.len(), 2);
        pretty_assertions::assert_eq!(pool.forks.forks_by_size.len(), 2);

        pretty_assertions::assert_eq!(
            pool.forks
                .forks_by_digest
                .get(&attestation_0.compound_digest())
                .unwrap(),
            &attestation_0
        );

        pretty_assertions::assert_eq!(
            pool.forks
                .forks_by_digest
                .get(&attestation_1.compound_digest())
                .unwrap(),
            &attestation_1
        );

        assert!(pool.forks.forks_by_size.contains(&KeySize {
            size: 1,
            height: 3,
            digest: attestation_0.compound_digest()
        }));

        assert!(pool.forks.forks_by_size.contains(&KeySize {
            size: 1,
            height: 3,
            digest: attestation_1.compound_digest()
        }));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_sanity_err_invalid_attestor(
        #[with([ATTESTOR_INVALID])] attestation: AttestationVote,
        config: Config,
    ) {
        let mut pool = AttestationPool::new(config);

        assert_matches::assert_matches!(
            pool.send(attestation.attestation.clone()),
            Err(Error::Unauthorized(ATTESTOR_INVALID, 2))
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

        let mut pool = AttestationPool::new(config);

        tokio_test::assert_pending!(tokio_test::task::spawn(pool.next()).poll());

        assert!(pool.send(attestation.attestation.clone()).is_ok());

        tokio_test::assert_ready_eq!(
            tokio_test::task::spawn(pool.next()).poll(),
            Some((quorum, permit, None))
        );
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

        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());

        let actual = pool.next().await;
        let expected = Some((quorum, permit, None));

        pretty_assertions::assert_eq!(actual, expected);
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

        let mut pool = AttestationPool::new(config);

        // Source chain height 1 (default)
        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());

        // Source chain height 101
        assert!(pool.send(attestation_3.attestation.clone()).is_ok());
        assert!(pool.send(attestation_4.attestation.clone()).is_ok());

        // NOTE: even though quorum 1 has LESS votes, it still passes the quorum threshold of 2.
        // The attestation pool always favors the HIGHEST quorum so as to improve catchup speed.

        let actual = pool.next().await;
        let expected = Some((quorum, permit, None));

        pretty_assertions::assert_eq!(actual, expected);
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
        let mut pool = AttestationPool::new(config);

        assert_matches::assert_matches!(
            pool.send(attestation_far_future.attestation.clone()),
            Err(Error::InvalidHeight(ATTESTOR_VALID_0, 501, 0))
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_note_attestation_finalization(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 1)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2)]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 3)]
        attestation_2: AttestationVote,
        config: Config,
    ) {
        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());

        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_0.attestation.header_number(),
            size: 1,
            digest: attestation_0.compound_digest()
        }));
        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_1.attestation.header_number(),
            size: 1,
            digest: attestation_1.compound_digest()
        }));
        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_2.attestation.header_number(),
            size: 1,
            digest: attestation_2.compound_digest()
        }));

        pool.note_attestation_finalization(stream::util::AttestationInfo {
            height: 2,
            ..Default::default()
        })
        .unwrap();

        assert!(!pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_0.attestation.header_number(),
            size: 1,
            digest: attestation_0.compound_digest()
        }));
        assert!(!pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_1.attestation.header_number(),
            size: 1,
            digest: attestation_1.compound_digest()
        }));
        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_2.attestation.header_number(),
            size: 1,
            digest: attestation_2.compound_digest()
        }));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_note_attestation_interval_change(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 1)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2)]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 3)]
        attestation_2: AttestationVote,
        config: Config,
    ) {
        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());

        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_0.attestation.header_number(),
            size: 1,
            digest: attestation_0.compound_digest()
        }));
        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_1.attestation.header_number(),
            size: 1,
            digest: attestation_1.compound_digest()
        }));
        assert!(pool.forks.forks_by_height.contains(&KeyHeight {
            height: attestation_2.attestation.header_number(),
            size: 1,
            digest: attestation_2.compound_digest()
        }));

        pool.note_attestation_interval_change(std::num::NonZero::new(1).unwrap());

        assert!(pool.forks.forks_by_digest.is_empty());
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_note_target_sample_size_shrinks(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 1)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 1)]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2)]
        attestation_2: AttestationVote,
        #[from(validate_quorum)]
        #[with(2)]
        _quorum_validate: ValidateQuorum,
        #[with(_quorum_validate.clone())] config: Config,
    ) {
        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());

        assert!(pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 1,
            size: 2,
            digest: attestation_0.compound_digest()
        }));
        assert!(!pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 2,
            size: 1,
            digest: attestation_2.compound_digest()
        }));

        pretty_assertions::assert_eq!(pool.forks.forks_best, Some(attestation_0.clone()));

        pool.note_target_sample_size_change(1);

        assert!(pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 1,
            size: 2,
            digest: attestation_0.compound_digest()
        }));
        assert!(pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 2,
            size: 1,
            digest: attestation_2.compound_digest()
        }));

        pretty_assertions::assert_eq!(pool.forks.forks_best, Some(attestation_2.clone()));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    async fn attestation_pool_note_target_sample_size_grows(
        _logs: (),
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 1)]
        attestation_0: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_1], 1)]
        attestation_1: AttestationVote,
        #[from(attestation)]
        #[with([ATTESTOR_VALID_0], 2)]
        attestation_2: AttestationVote,
        #[from(validate_quorum)]
        #[with(1)]
        _quorum_validate: ValidateQuorum,
        #[with(_quorum_validate.clone())] config: Config,
    ) {
        let mut pool = AttestationPool::new(config);

        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());

        assert!(pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 1,
            size: 2,
            digest: attestation_0.compound_digest()
        }));
        assert!(pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 2,
            size: 1,
            digest: attestation_2.compound_digest()
        }));

        pretty_assertions::assert_eq!(pool.forks.forks_best, Some(attestation_2.clone()));

        pool.note_target_sample_size_change(2);

        assert!(pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 1,
            size: 2,
            digest: attestation_0.compound_digest()
        }));
        assert!(!pool.forks.quorums_by_height.contains(&KeyHeight {
            height: 2,
            size: 1,
            digest: attestation_2.compound_digest()
        }));

        pretty_assertions::assert_eq!(pool.forks.forks_best, Some(attestation_0));
    }

    #[tokio::test]
    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    #[allow(clippy::too_many_arguments)]
    async fn attestation_pool_note_chain_reversion(
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

        let mut pool = AttestationPool::new(config);

        // ------------------------------------------------------------------------
        // 1) Create a quorum and mark it for later.
        //    This populates:
        //      - valid.quorums_valid
        //      - digest_local
        // ------------------------------------------------------------------------
        assert!(pool.send(attestation_0.attestation.clone()).is_ok());
        assert!(pool.send(attestation_1.attestation.clone()).is_ok());

        let (_quorum_high, permit_0, _digest_local) = pool.next().await.unwrap();

        let attestation_signed_0 = common::types::AttestationSigned {
            attestation: attestation_0.attestation.attestation_data.clone(),
            signature: [0u8; 96],
            attestors: vec![
                attestation_0.attestation.attestor.clone(),
                attestation_1.attestation.attestor.clone(),
            ],
            continuity_proof: attestation_0.attestation.continuity_proof.clone(),
        };

        pool.mark_for_later(
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
        assert!(pool.send(attestation_2.attestation.clone()).is_ok());
        assert!(pool.send(attestation_3.attestation.clone()).is_ok());

        let (_quorum_low, permit_1, _digest_local) = pool.next().await.unwrap();
        pool.mark_invalid(permit_1);

        // ------------------------------------------------------------------------
        // 3) Create another quorum and leave it in forks.
        // This populates forks_by_digest / forks_by_height / forks_by_size / quorums_by_height / votes
        // ------------------------------------------------------------------------
        assert!(pool.send(attestation_4.attestation.clone()).is_ok());
        assert!(pool.send(attestation_5.attestation.clone()).is_ok());

        // ------------------------------------------------------------------------
        // 4) Add a pending attestation.
        //    This populates:
        //      - pending_by_digest / pending_by_prev_digest_tail / pending_by_height
        //      - attestation_delay.time
        // ------------------------------------------------------------------------
        assert!(pool.send(attestation_pending.attestation.clone()).is_ok());

        // Sanity-check that we actually populated the structures before reversion.
        assert!(pool.digest_local.is_some());

        assert!(!pool.forks.forks_by_digest.is_empty());
        assert!(!pool.forks.forks_by_height.is_empty());
        assert!(!pool.forks.forks_by_size.is_empty());
        assert!(pool.forks.forks_best.is_some());

        assert!(!pool.forks.pending_by_digest.is_empty());
        assert!(!pool.forks.pending_by_prev_digest_tail.is_empty());
        assert!(!pool.forks.pending_by_height.is_empty());

        assert!(!pool.forks.votes.is_empty());
        assert!(!pool.forks.votes_invalid.is_empty());
        assert!(!pool.forks.quorums_by_height.is_empty());

        assert!(!pool.valid.quorums_valid.is_empty());
        assert!(!pool.attestation_delay.time.is_empty());

        // ------------------------------------------------------------------------
        // 5) Revert the chain and verify everything is cleared/reset.
        // ------------------------------------------------------------------------
        let reversion_info = stream::util::AttestationInfo {
            height: 50,
            digest: DIGEST_1.digest,
        };

        pool.note_attestation_chain_reversion(reversion_info);

        // Digest local reset
        pretty_assertions::assert_eq!(pool.digest_local, None);

        // Forks reset
        assert!(pool.forks.forks_by_digest.is_empty());
        assert!(pool.forks.forks_by_height.is_empty());
        assert!(pool.forks.forks_by_size.is_empty());
        pretty_assertions::assert_eq!(pool.forks.forks_best, None);

        assert!(pool.forks.pending_by_digest.is_empty());
        assert!(pool.forks.pending_by_prev_digest_tail.is_empty());
        assert!(pool.forks.pending_by_height.is_empty());

        assert!(pool.forks.votes.is_empty());
        assert!(pool.forks.votes_invalid.is_empty());
        assert!(pool.forks.quorums_by_height.is_empty());

        // Reversion should set the new finalized digest
        pretty_assertions::assert_eq!(pool.forks.last_finalized_digest, Some(DIGEST_1.digest));

        // Valid queue reset
        assert!(pool.valid.quorums_valid.is_empty());

        // Delay tracking reset
        assert!(pool.attestation_delay.time.is_empty());
    }

    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    fn validate_height_rejects_beyond_catchup_window(
        #[with(2, 10, 0, 5)] validate_quorum: ValidateQuorum,
    ) {
        // With no finalization, upper bound = start_height + 50 = 50
        assert!(validate_quorum.validate_height(50, None));
        assert!(!validate_quorum.validate_height(60, None));

        // With finalization at height 20, upper bound = 20 + 50 = 70
        assert!(validate_quorum.validate_height(70, Some(20)));
        assert!(!validate_quorum.validate_height(80, Some(20)));
    }

    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    fn validate_height_rejects_at_or_below_finalized(
        #[with(2, 10, 0, 500)] validate_quorum: ValidateQuorum,
    ) {
        // Height at finalized is rejected
        assert!(!validate_quorum.validate_height(20, Some(20)));
        // Height below finalized is rejected
        assert!(!validate_quorum.validate_height(10, Some(20)));
        // First valid height above finalized
        assert!(validate_quorum.validate_height(30, Some(20)));
    }

    #[rstest::rstest]
    #[timeout(TIMEOUT)]
    fn validate_height_accepts_start_height_when_no_finalization(
        #[with(2, 10, 100, 500)] validate_quorum: ValidateQuorum,
    ) {
        // Before any finalization, start_height is valid
        assert!(validate_quorum.validate_height(100, None));
        // Below start_height is rejected
        assert!(!validate_quorum.validate_height(90, None));
        // Above start_height and aligned is valid
        assert!(validate_quorum.validate_height(110, None));
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
}
