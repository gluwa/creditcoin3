#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "runtime-benchmarks"))]
mod continuity_dev;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

mod asset;
mod continuity;
mod impls;
mod ledger;

#[frame_support::pallet]
pub mod pallet {
    use core::marker::PhantomData;

    use crate::ledger::AttestorLedger;
    use attestor_primitives::{
        provider::{AttestationProvider, CheckpointProvider},
        AttestationChainConfiguration, AttestationCheckpoint, Attestor, BlsPublicKey,
        BlsPublicKeyWrapper, BlsSignature, ChainAttestationIntervalType, ChainEncodingVersion,
        ChainKey, Digest, SignedAttestation,
    };
    use frame_support::{
        dispatch::{ClassifyDispatch, DispatchClass, Pays, PaysFee, WeighData},
        pallet_prelude::{OptionQuery, ValueQuery, *},
        traits::{Currency, LockableCurrency, OnUnbalanced},
        Blake2_128Concat, Twox64Concat,
    };
    use frame_system::pallet_prelude::*;
    use parity_scale_codec::FullCodec;
    use sp_staking::StakingInterface;
    use sp_std::collections::{btree_set::BTreeSet, vec_deque::VecDeque};
    use sp_std::{fmt::Debug, vec::Vec};
    use supported_chains_primitives::provider::{OnRegisterChainProvider, SupportedChainsProvider};

    pub const MAX_CHECKPOINTS_CLEARED_PER_BLOCK: u8 = 40;

    // Amount of blocks tracked in a single checkpoint bucket
    pub const CHECKPOINT_BUCKET_SIZE: u64 = 1000;

    /// The balance type of this pallet.
    pub type BalanceOf<T> = <T as Config>::CurrencyBalance;
    pub type PositiveImbalanceOf<T> = <<T as Config>::Currency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::PositiveImbalance;

    /// The election policy used when electing new attestors after each epoch.
    #[derive(
        PartialEq, Eq, Copy, Clone, Encode, Decode, RuntimeDebug, TypeInfo, MaxEncodedLen, Default,
    )]
    pub enum AttestorElectionPolicy {
        /// Any attestor can be selected.
        #[default]
        OpenToAny = 0,
        /// Only authorized attestors can be selected.
        AuthorizedOnly = 1,
        /// No new attestors can be selected.
        DeniedToAll = 2,
    }

    // We define this type alias so that clippy doesn't complain about the unused unit type.
    type AuthorizationValue = ();

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_balances::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        // TODO: when updating polkadot-sdk we should use `InspectLockableCurrency`
        type Currency: LockableCurrency<
            Self::AccountId,
            Moment = BlockNumberFor<Self>,
            Balance = Self::CurrencyBalance,
        >;
        /// Just the `Currency::Balance` type; we have this item to allow us to constrain it to
        /// `From<u128>`.
        type CurrencyBalance: sp_runtime::traits::AtLeast32BitUnsigned
            + FullCodec
            + Copy
            + MaybeSerializeDeserialize
            + core::fmt::Debug
            + Default
            + From<u64>
            + From<u128>
            + TypeInfo
            + MaxEncodedLen;
        #[pallet::constant]
        type DefaultAttestationsPerCheckpoint: Get<u32>;
        #[pallet::constant]
        type DefaultAttestationInterval: Get<ChainAttestationIntervalType>;
        #[pallet::constant]
        type DefaultTargetSampleSize: Get<u32>;
        /// The default maximum catchup bound, expressed in **blocks**.
        /// When an attestation chain falls behind the source chain (e.g.
        /// during bootstrap or after a network stall), attestors produce
        /// larger-than-usual attestations whose continuity proofs span
        /// more blocks. To prevent unbounded proof sizes from overwhelming
        /// runtime validation (risking an execution chain stall), this
        /// parameter caps the continuity proof size: each catchup
        /// attestation covers **at most** this many blocks.
        #[pallet::constant]
        type DefaultMaxCatchup: Get<u32>;
        #[pallet::constant]
        type MaxAttestationNodes: Get<u32>;
        // TODO: Make this useful
        #[pallet::constant]
        type CommittmentInterval: Get<u64>;
        #[pallet::constant]
        type DefaultMinBondRequirement: Get<u128>;
        #[pallet::constant]
        type MaxUnlockingChunks: Get<u32>;
        /// Number of eras that staked funds must remain bonded for.
        #[pallet::constant]
        type BondingDuration: Get<u32>;
        /// Default duration in number of attestations for which we keep attestations after they are condensed in a checkpoint.
        #[pallet::constant]
        type DefaultAttestationRetentionDuration: Get<u32>;
        /// The access to staking functionality.
        type Staking: StakingInterface<Balance = BalanceOf<Self>, AccountId = Self::AccountId>;
        /// Handler for the unbalanced increment when rewarding a staker.
        /// NOTE: in most cases, the implementation of `OnUnbalanced` should modify the total
        /// issuance.
        type Reward: OnUnbalanced<PositiveImbalanceOf<Self>>;

        /// The type of the BLS aggregated signature
        type BlsSignature: FullCodec
            + Clone
            + Debug
            + PartialEq
            + Eq
            + Send
            + Sync
            + TypeInfo
            + MaxEncodedLen
            + From<[u8; 42]>;

        type SupportedChains: SupportedChainsProvider;

        #[pallet::constant]
        type MaxAttestationsPerBlock: Get<u32>;
        #[pallet::constant]
        type MaxCheckpointsImportedPerCall: Get<u32>;
        #[pallet::constant]
        type DefaultAttestationChainGenesisBlockNumber: Get<u64>;
        /// Origin that can perform Operator-only calls
        type OperatorsOrigin: EnsureOrigin<Self::RuntimeOrigin>;
    }

    pub trait WeightInfo {
        fn register_attestor() -> Weight;
        fn unregister_attestor() -> Weight;
        fn set_max_attestors() -> Weight;
        fn register_invulnerable() -> Weight;
        fn unregister_invulnerable() -> Weight;
        fn set_max_invulnerables() -> Weight;
        fn bootstrap_chain(a: u32) -> Weight;
        fn commit_attestation(a: u32, b: u32) -> Weight;
        fn set_target_sample_size() -> Weight;
        fn set_chain_attestation_interval() -> Weight;
        fn set_attestations_per_checkpoint() -> Weight;
        fn set_min_bond_requirement() -> Weight;
        fn chill() -> Weight;
        fn attest() -> Weight;
        fn withdraw_unbonded() -> Weight;
        fn on_initialize(a: u32) -> Weight;
        fn import_checkpoints() -> Weight;
        fn set_attestation_chain_genesis_block_number() -> Weight;
        fn set_election_policy() -> Weight;
        fn authorize_attestor() -> Weight;
        fn remove_authorized_attestor() -> Weight;
        fn kick_active_attestor() -> Weight;
        fn force_election() -> Weight;
        fn set_max_catchup() -> Weight;
        fn force_apply_updates() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn attestors)]
    // Attestor storage maps a "Stash account" to an Attestor (controller)
    // Per supported chain
    pub type Attestors<T: Config> = StorageDoubleMap<
        _,
        Twox64Concat,
        ChainKey,
        Blake2_128Concat,
        T::AccountId,
        Attestor<T::AccountId>,
    >;

    #[pallet::storage]
    #[pallet::getter(fn active_attestor_set)]
    // Active attestors are the ones that have been registered and are not in the chilling state
    pub type ActiveAttestors<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, Vec<T::AccountId>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn invulnerables)]
    pub type Invulnerables<T: Config> =
        StorageDoubleMap<_, Twox64Concat, ChainKey, Blake2_128Concat, T::AccountId, bool>;

    #[pallet::storage]
    #[pallet::getter(fn max_attestors)]
    pub type MaxAttestors<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, u32, ValueQuery, MaxAttestorsDefault<T>>;

    #[pallet::storage]
    #[pallet::getter(fn max_invulnerables)]
    pub type MaxInvulnerables<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, u32, ValueQuery, MaxInvulernablesDefault<T>>;

    #[pallet::type_value]
    pub fn MaxAttestorsDefault<T: Config>() -> u32 {
        T::MaxAttestationNodes::get()
    }

    #[pallet::type_value]
    pub fn MaxInvulernablesDefault<T: Config>() -> u32 {
        T::MaxAttestationNodes::get()
    }

    #[pallet::storage]
    #[pallet::getter(fn attestations)]
    pub type Attestations<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        ChainKey,
        Blake2_128Concat,
        Digest,
        SignedAttestation<T::Hash, T::AccountId>,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn checkpoints)]
    pub type Checkpoints<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, ChainKey, Identity, u64, Digest, OptionQuery>;

    #[pallet::storage]
    pub type CheckpointBuckets<T: Config> = StorageNMap<
        _,
        (
            NMapKey<Blake2_128Concat, ChainKey>, // ChainKey
            NMapKey<Blake2_128Concat, u64>,      // Block number pivot see: CHECKPOINT_BUCKET_SIZE
            NMapKey<Blake2_128Concat, u64>,      // Block number
        ),
        (),
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn last_checkpoint)]
    pub type LastCheckpoint<T> = StorageMap<_, Blake2_128Concat, ChainKey, AttestationCheckpoint>;

    #[pallet::storage]
    #[pallet::getter(fn checkpointing_queues)]
    pub type CheckpointingQueues<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, VecDeque<Digest>, ValueQuery, GetDefault>;

    #[pallet::storage]
    #[pallet::getter(fn last_attestation_digest)]
    pub type LastDigest<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, (u64, Digest), OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn pending_target_sample_size)]
    pub type PendingTargetSampleSize<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, u32, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn target_sample_size)]
    pub type TargetSampleSize<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, u32, ValueQuery, TargetSampleSizeDefault<T>>;

    #[pallet::type_value]
    pub fn TargetSampleSizeDefault<T: Config>() -> u32 {
        T::DefaultTargetSampleSize::get()
    }

    #[pallet::storage]
    #[pallet::getter(fn chain_attestation_interval)]
    pub type ChainAttestationInterval<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainKey,
        ChainAttestationIntervalType,
        ValueQuery,
        AttestationIntervalDefault<T>,
    >;

    #[pallet::type_value]
    pub fn AttestationIntervalDefault<T: Config>() -> ChainAttestationIntervalType {
        T::DefaultAttestationInterval::get()
    }

    #[pallet::storage]
    #[pallet::getter(fn pending_attestation_interval)]
    pub type PendingAttestationInterval<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, ChainAttestationIntervalType, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn attestation_checkpoint_interval)]
    pub type AttestationCheckpointInterval<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainKey,
        u32,
        ValueQuery,
        DefaultAttestationsPerCheckpoint<T>,
    >;

    #[pallet::type_value]
    pub fn DefaultAttestationsPerCheckpoint<T: Config>() -> u32 {
        T::DefaultAttestationsPerCheckpoint::get()
    }

    /// The maximum catchup bound (in **blocks**) per chain. During catchup,
    /// each attestation's continuity proof will span **at most** this many
    /// blocks, preventing unbounded proof sizes from stalling the
    /// execution chain.
    #[pallet::storage]
    #[pallet::getter(fn max_catchup)]
    pub type MaxCatchup<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, u32, ValueQuery, MaxCatchupDefault<T>>;

    #[pallet::type_value]
    pub fn MaxCatchupDefault<T: Config>() -> u32 {
        T::DefaultMaxCatchup::get()
    }

    #[pallet::storage]
    #[pallet::getter(fn pending_max_catchup)]
    pub type PendingMaxCatchup<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, u32, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn min_bond_requirement)]
    pub type MinBondRequirement<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainKey,
        BalanceOf<T>,
        ValueQuery,
        DefaultMinBondRequirement<T>,
    >;

    #[pallet::type_value]
    pub fn DefaultMinBondRequirement<T: Config>() -> BalanceOf<T> {
        T::DefaultMinBondRequirement::get().into()
    }

    /// Map from all (unlocked) "controller" accounts to info regarding staking.
    ///
    /// Note: All the reads and mutations to this storage *MUST* be done through the methods exposed
    /// by [`AttestorLedger`] to ensure data and lock consistency.
    #[pallet::storage]
    #[pallet::getter(fn ledger)]
    pub type Ledger<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, AttestorLedger<T>>;

    /// Progress markers for removing the checkpoints associated with source chains that are
    /// no longer supported. Maps from a chain_key to a cursor representing the point up to which
    /// that chain's checkpoints have been removed.
    #[pallet::storage]
    #[pallet::getter(fn checkpoint_clearing_cursors)]
    pub type CheckpointClearingCursors<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, Vec<u8>, OptionQuery>;

    /// The duration in number of attestations for which we keep attestations that have already been
    /// condensed in a checkpoint. Keeping these for a time ensures that proofs generated using the
    /// attestations in question remain verifyable long enough to be submitted on-chain.
    #[pallet::storage]
    #[pallet::getter(fn attestation_retention_duration)]
    pub type AttestationRetentionDuration<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainKey,
        u32,
        ValueQuery,
        DefaultAttestationRetentionDuration<T>,
    >;

    #[pallet::type_value]
    pub fn DefaultAttestationRetentionDuration<T: Config>() -> u32 {
        T::DefaultAttestationRetentionDuration::get()
    }

    /// A queue containing the digests of attestations to be removed from storage. When the queue fills beyond
    /// AttestationRetentionDuration, we remove attestations from the queue and from the Attestations storage
    /// map.
    #[pallet::storage]
    #[pallet::getter(fn attestation_removal_queue)]
    pub type AttestationRemovalQueues<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, VecDeque<Digest>, ValueQuery, GetDefault>;

    #[pallet::type_value]
    pub fn AttestationChainGenesisBlockNumberDefault<T: Config>() -> u64 {
        T::DefaultAttestationChainGenesisBlockNumber::get()
    }

    /// The genesis block number for the attestation chain.
    /// This is used to determine the starting point for the attestation chain.
    #[pallet::storage]
    #[pallet::getter(fn attestation_chain_genesis_block_number)]
    pub type AttestationChainGenesisBlockNumber<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainKey,
        u64,
        ValueQuery,
        AttestationChainGenesisBlockNumberDefault<T>,
    >;

    /// The current election policy for each chain.
    /// Represents the policy used when electing new attestors after each epoch.
    #[pallet::storage]
    #[pallet::getter(fn chain_election_policy)]
    pub type ChainElectionPolicy<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, AttestorElectionPolicy, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn authorized_attestors)]
    #[allow(clippy::unused_unit)]
    // Authorized attestors are a subset of attestors that can be elected when the election policy is AuthorizedOnly
    pub type AuthorizedAttestors<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        ChainKey,
        Blake2_128Concat,
        T::AccountId,
        AuthorizationValue,
        ValueQuery,
    >;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub invulnerables: Vec<(T::AccountId, BlsPublicKeyWrapper)>,
        pub attestation_chain_configurations: Vec<AttestationChainConfiguration>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            for chain_configuration in self.attestation_chain_configurations.iter() {
                // Set the committee set size for the chain
                TargetSampleSize::<T>::insert(
                    chain_configuration.chain_key,
                    chain_configuration.target_sample_size,
                );

                ChainAttestationInterval::<T>::insert(
                    chain_configuration.chain_key,
                    chain_configuration.attestation_interval,
                );
                AttestationCheckpointInterval::<T>::insert(
                    chain_configuration.chain_key,
                    chain_configuration.attestations_per_checkpoint,
                );

                MaxAttestors::<T>::insert(
                    chain_configuration.chain_key,
                    T::MaxAttestationNodes::get(),
                );

                MaxInvulnerables::<T>::insert(
                    chain_configuration.chain_key,
                    T::MaxAttestationNodes::get(),
                );

                for invulnerable in self.invulnerables.iter() {
                    Invulnerables::<T>::insert(
                        chain_configuration.chain_key,
                        invulnerable.0.clone(),
                        true,
                    );
                }

                // Track the checkpoint with the highest block_number for LastCheckpoint
                let mut last_checkpoint: Option<AttestationCheckpoint> = None;

                for checkpoint in chain_configuration.checkpoints.iter() {
                    Checkpoints::<T>::insert(
                        chain_configuration.chain_key,
                        checkpoint.block_number,
                        checkpoint.digest,
                    );
                    CheckpointBuckets::<T>::insert(
                        (
                            chain_configuration.chain_key,
                            Pallet::<T>::compute_block_index_for(checkpoint.block_number),
                            checkpoint.block_number,
                        ),
                        (),
                    );

                    // Only update last_checkpoint if this checkpoint has a higher block_number
                    if last_checkpoint
                        .as_ref()
                        .is_none_or(|lc| checkpoint.block_number >= lc.block_number)
                    {
                        last_checkpoint = Some(checkpoint.clone());
                    }
                }
                if let Some(checkpoint) = last_checkpoint {
                    LastCheckpoint::<T>::insert(chain_configuration.chain_key, checkpoint);
                }
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Emitted when an attestor is properly registered with the attestation system
        AttestorRegistered(ChainKey, T::AccountId),
        AttestorUnregistered(ChainKey, T::AccountId),
        /// Emitted when an invulnerable is properly registered with the attestation system
        InvulnerableRegistered(ChainKey, T::AccountId),
        InvulnerableUnregistered(ChainKey, T::AccountId),
        BlockAttested(ChainKey, u64, Digest),
        CheckpointReached(ChainKey, AttestationCheckpoint),
        PendingTargetSampleSizeSet(ChainKey, u32),
        TargetSampleSizeChanged(ChainKey, u32),
        Bonded {
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        Unbonded {
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        Withdrawn {
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        AttestorActivated(ChainKey, T::AccountId, BlsPublicKey),
        AttestorChilled(ChainKey, T::AccountId),
        AttestorsElected {
            epoch: u64,
            chain_key: ChainKey,
            attestors: Vec<T::AccountId>,
        },
        MinBondRequirementUpdated(ChainKey, BalanceOf<T>),

        /// Note a change in the attestation interval for a source chain. Also notes the
        /// block number of the latest attestation for that source chain at the time of
        /// the interval change.
        AttestationIntervalChanged(ChainKey, ChainAttestationIntervalType),
        PendingAttestationIntervalSet(ChainKey, ChainAttestationIntervalType),
        /// Signals that checkpoints were cleared for a chain that is no longer supported.
        /// A fixed number of checkpoints will be cleared per block until none remain.
        CheckpointsCleared(ChainKey),
        CheckpointIntervalChanged(ChainKey, u32),
        MaxCatchupChanged(ChainKey, u32),
        PendingMaxCatchupSet(ChainKey, u32),
        /// A source chain was removed via pallet supported chains. Associated storage
        /// in pallet attestation was cleaned up.
        ClearedStorageForRemovedChain(ChainKey),
        /// Max attestors changed for a chain
        MaxAttestorsChanged(ChainKey, u32),
        /// Attestation chain genesis block number was set for a chain.
        AttestationChainGenesisBlockNumberSet(ChainKey, u64),
        /// Note a change in the attestor election policy.
        ChangedElectionPolicy(ChainKey, AttestorElectionPolicy),
        /// An attestor was authorized for a specific chain.
        AuthorizedAttestorAdded(ChainKey, T::AccountId),
        /// An attestor was unauthorized for a specific chain.
        AuthorizedAttestorRemoved(ChainKey, T::AccountId),
        /// A force election was triggered via sudo.
        ForcedElection {
            epoch: u64,
        },
        /// Pending updates were force-applied via operator call.
        ForcedUpdatesApplied,
    }

    #[pallet::error]
    pub enum Error<T> {
        /// The AccountId supplied has already been registered
        AlreadyAttestor,
        /// The attestor list is at the max size allowed by the current configuration
        AttestorListFull,
        /// the address supplied is not currently registered as an attestor
        AddressNotAttestor,
        /// The invulnerable list is full
        InvulnerableListFull,
        /// The call to set_max_invulnerables, most likely because the current list is longer than the new requested maximum
        MaxInvulnerablesCannotBeChanged,
        /// The call the urnegister_invulnerable failed because the address is not invulnerable
        AddressIsNotInvulnerable,
        /// The call to attest_block failed, the block's cryptographic committments were invalid
        InvalidAttestation,
        // If there is no digest stored yet
        NoPreviousDigest,
        // If there is a duplicate attestation
        AttestationExists,
        /// The chain is not supported
        ChainNotSupported,
        // Bls public key is invalid
        InvalidBlsPublicKey,
        // Invalid BLS signature
        InvalidBlsSignature,
        // Failed proof of possession check
        InvalidProofOfPossession,
        // Accessed non-existant storage entry, in checkpointing queue
        // or attestations. This error should not occur unless there
        // is a bug in this pallet.
        CheckpointCreationError,
        // The last checkpoint is empty when it is not expected to be
        LastCheckpointEmpty,
        // Checkpoint width calculated as zero which should be impossible
        CheckpointWidthIsZero,
        // Not enough attestations to create a checkpoint found in the checkpointing queue
        CheckpointingQueueDrained,
        // The attestation referenced in the queue points to a non-existent attestation
        AttestationNotFound,
        // The attestation in which the target checkpoint block should have been
        // doesn't contain the expected block number
        CheckpointTargetNotFound,
        // Invalid attestor account
        InvalidAttestorAccount,
        // Insufficient balance to bond
        InsufficientBalance,
        // Not a stash account
        NotStash,
        // No more unlock chunks
        NoMoreChunks,
        // Not your attestor
        NotYourAttestor,
        // Already bonded
        AlreadyBonded,
        // Attestor is not in idle state
        AttestorNotIdle,
        // No supported chains
        NoSupportedChains,
        // Tried to set attestation interval to an invalid value.
        InvalidAttestationInterval,
        // Tried to set attestations per checkpoint to an invalid value.
        InvalidAttestationsPerCheckpoint,
        InvalidMaxCatchup,
        // Tried to set committee set size to an invalid value.
        InvalidTargetSampleSize,
        // Tried to import checkpoints for chain key that already has attestations.
        AttestationFoundWhileImporting,
        // Invalid attestation chain block number.
        InvalidAttestationBlockNumber,
        // Errors when validating an attestation
        // Invalid attestor account
        InvalidAttestorFound,
        // Attestor is not active
        AttestorNotActive,
        // BLS public key is invalid
        AttestorWithInvalidPublicKey,
        // Majority of signatures not reached
        MajorityNotReached,
        // Duplicate attestor in signatures
        DuplicateAttestor,
        // Attestor is already authorized for the chain.
        AttestorAlreadyAuthorized,
        // Attestor is not authorized for the chain.
        AttestorNotAuthorized,
        // No finalized attestation found when one is required
        NoFinalizedAttestation,
        // Cannot set genesis block number when attestations or checkpoints already exist for the chain
        AttestationsAlreadyExist,
        // Continuity proof is empty when it shouldn't be
        EmptyContinuityProof,
        // Continuity proof is invalid
        InvalidAttestationContinuityProof,
        // Continuity proof tail does not link to last finalized attestation
        InvalidAttestationContinuityProofTail,
        // Continuity proof head does not link to attestation prev_digest
        InvalidAttestationContinuityProofHead,
        // Continuity proof has a bad block link
        InvalidAttestationContinuityProofBlock,
        // Invalid genesis block in continuity proof
        InvalidAttestationContinuityProofBlockGenesis,
        // Attestation previous digest is invalid
        InvalidAttestationPrevDigest,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Initialization
        fn on_initialize(_now: BlockNumberFor<T>) -> Weight {
            if let Some((chain_key, cursor)) = CheckpointClearingCursors::<T>::iter().next() {
                let maybe_cursor = Checkpoints::<T>::clear_prefix(
                    chain_key,
                    u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                    Some(&cursor[..]),
                )
                .maybe_cursor;
                CheckpointClearingCursors::<T>::set(chain_key, maybe_cursor);

                // note: may be triggered multiple times when removing a large amount of checkpoints
                Self::deposit_event(Event::<T>::CheckpointsCleared(chain_key));

                // Cleared checkpoints for 1 chain
                <T as Config>::WeightInfo::on_initialize(1)
            } else {
                // Cleared checkpoints for 0 chains
                <T as Config>::WeightInfo::on_initialize(0)
            }
        }
    }

    /// Deprecation notice: The extrinsics with indexes 12, 15 and 17 have been removed.
    /// The functionality of these extrinsics has been eliminated.
    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::set_chain_attestation_interval())]
        pub fn set_chain_attestation_interval(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            chain_attestation_interval: ChainAttestationIntervalType,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure! {
                chain_attestation_interval > 0,
                Error::<T>::InvalidAttestationInterval
            };

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );

            PendingAttestationInterval::<T>::set(chain_key, Some(chain_attestation_interval));

            Self::deposit_event(Event::<T>::PendingAttestationIntervalSet(
                chain_key,
                chain_attestation_interval,
            ));

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::set_target_sample_size())]
        pub fn set_target_sample_size(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            new_target_sample_size: u32,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure! {
                new_target_sample_size > 0,
                Error::<T>::InvalidTargetSampleSize
            };

            PendingTargetSampleSize::<T>::set(chain_key, Some(new_target_sample_size));

            Self::deposit_event(Event::<T>::PendingTargetSampleSizeSet(
                chain_key,
                new_target_sample_size,
            ));

            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::register_attestor())]
        pub fn register_attestor(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::try_insert_attestor_and_emit_event(chain_key, who, attestor_id)
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_attestor())]
        pub fn unregister_attestor(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::remove_attestor_and_emit_event(chain_key, who, attestor_id)?;

            Ok(())
        }

        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_attestors())]
        pub fn set_max_attestors(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            new_max: u32,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            MaxAttestors::<T>::insert(chain_key, new_max);

            Self::deposit_event(Event::<T>::MaxAttestorsChanged(chain_key, new_max));

            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::register_invulnerable())]
        pub fn register_invulnerable(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor: T::AccountId,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            Self::try_insert_invulnerable_and_emit_event(chain_key, &attestor)
        }

        #[pallet::call_index(6)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_invulnerable())]
        pub fn unregister_invulnerable(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor: T::AccountId,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure!(
                Self::address_is_invulnerable(chain_key, &attestor),
                Error::<T>::AddressIsNotInvulnerable
            );

            Self::remove_invulnerable_and_emit_event(chain_key, attestor)
        }

        #[pallet::call_index(7)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_invulnerables())]
        pub fn set_max_invulnerables(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            new_max: u32,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            let count = Invulnerables::<T>::iter_prefix_values(chain_key)
                .collect::<Vec<_>>()
                .len() as u32;

            ensure!(
                new_max >= count,
                Error::<T>::MaxInvulnerablesCannotBeChanged
            );

            MaxInvulnerables::<T>::insert(chain_key, new_max);
            Ok(())
        }

        #[pallet::call_index(8)]
        #[pallet::weight(<T as Config>::WeightInfo::bootstrap_chain(attestation.attestors.len() as u32))]
        pub fn bootstrap_chain(
            origin: OriginFor<T>,
            attestation: SignedAttestation<T::Hash, T::AccountId>,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            Self::do_bootstrap_chain(attestation)
        }

        /// [`CommitAttestationWeight`] makes it so active attestors do not pay fees on this
        /// extrinsic
        #[pallet::call_index(9)]
        #[pallet::weight(CommitAttestationWeight::<T>::default())]
        pub fn commit_attestation(
            origin: OriginFor<T>,
            attestation: SignedAttestation<T::Hash, T::AccountId>,
        ) -> DispatchResult {
            // Only allow active attestors to commit attestations
            let account = ensure_signed(origin)?;
            let chain_key = attestation.chain_key();
            let active_attestors = ActiveAttestors::<T>::get(chain_key)
                .into_iter()
                .collect::<BTreeSet<_>>();
            ensure!(
                active_attestors.contains(&account),
                Error::<T>::AttestorNotActive
            );

            Self::do_commit_attestation(attestation)
        }

        #[pallet::call_index(10)]
        #[pallet::weight(<T as Config>::WeightInfo::set_attestations_per_checkpoint())]
        pub fn set_attestations_per_checkpoint(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestations_per_checkpoint: u32,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure! {
                attestations_per_checkpoint > 0,
                Error::<T>::InvalidAttestationsPerCheckpoint
            };

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );

            AttestationCheckpointInterval::<T>::set(chain_key, attestations_per_checkpoint);
            Self::deposit_event(Event::<T>::CheckpointIntervalChanged(
                chain_key,
                attestations_per_checkpoint,
            ));

            Ok(())
        }

        #[pallet::call_index(11)]
        #[pallet::weight(<T as Config>::WeightInfo::set_min_bond_requirement())]
        pub fn set_min_bond_requirement(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            min_bond_requirement: BalanceOf<T>,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            MinBondRequirement::<T>::set(chain_key, min_bond_requirement);

            Self::deposit_event(Event::<T>::MinBondRequirementUpdated(
                chain_key,
                min_bond_requirement,
            ));

            Ok(())
        }

        #[pallet::call_index(13)]
        #[pallet::weight(<T as Config>::WeightInfo::attest())]
        pub fn attest(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            bls_public_key: BlsPublicKey,
            proof_of_possession: BlsSignature,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::start_attesting(chain_key, who, bls_public_key, proof_of_possession)?;

            Ok(())
        }

        #[pallet::call_index(14)]
        #[pallet::weight(<T as Config>::WeightInfo::chill())]
        pub fn chill(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let attestor = Attestors::<T>::get(chain_key, &attestor_id)
                .ok_or(Error::<T>::AddressNotAttestor)?;

            // Only chill your own attestor
            ensure!(attestor.stash == who, Error::<T>::NotYourAttestor);

            Self::do_chill_attestor(chain_key, attestor_id);

            Ok(())
        }

        #[pallet::call_index(16)]
        #[pallet::weight(<T as Config>::WeightInfo::withdraw_unbonded())]
        pub fn withdraw_unbonded(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::do_withdraw_unbonded(&who)?;

            Ok(())
        }

        #[pallet::call_index(18)]
        #[pallet::weight(<T as Config>::WeightInfo::import_checkpoints())]
        pub fn import_checkpoints(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            checkpoints: BoundedVec<AttestationCheckpoint, T::MaxCheckpointsImportedPerCall>,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            Self::do_import_checkpoints(chain_key, checkpoints)?;

            Ok(())
        }

        #[pallet::call_index(19)]
        #[pallet::weight(<T as Config>::WeightInfo::set_attestation_chain_genesis_block_number())]
        pub fn set_attestation_chain_genesis_block_number(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            genesis_block_number: u64,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );

            // Ensure no attestations or checkpoints exist for this chain before setting genesis block number
            ensure!(
                Attestations::<T>::iter_prefix(chain_key).next().is_none()
                    && Checkpoints::<T>::iter_prefix(chain_key).next().is_none(),
                Error::<T>::AttestationsAlreadyExist
            );

            // Set the genesis block number for the attestation chain
            AttestationChainGenesisBlockNumber::<T>::insert(chain_key, genesis_block_number);

            Self::deposit_event(Event::<T>::AttestationChainGenesisBlockNumberSet(
                chain_key,
                genesis_block_number,
            ));

            Ok(())
        }

        #[pallet::call_index(21)]
        #[pallet::weight(<T as Config>::WeightInfo::set_election_policy())]
        pub fn set_election_policy(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            new_policy: AttestorElectionPolicy,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );

            ChainElectionPolicy::<T>::insert(chain_key, new_policy);

            Self::deposit_event(Event::<T>::ChangedElectionPolicy(chain_key, new_policy));

            Ok(())
        }

        #[pallet::call_index(22)]
        #[pallet::weight(<T as Config>::WeightInfo::authorize_attestor())]
        pub fn authorize_attestor(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );
            ensure!(
                Attestors::<T>::contains_key(chain_key, &attestor_id),
                Error::<T>::AddressNotAttestor
            );
            ensure!(
                !AuthorizedAttestors::<T>::contains_key(chain_key, &attestor_id),
                Error::<T>::AttestorAlreadyAuthorized
            );

            AuthorizedAttestors::<T>::insert(chain_key, attestor_id.clone(), ());

            Self::deposit_event(Event::<T>::AuthorizedAttestorAdded(chain_key, attestor_id));

            Ok(())
        }

        #[pallet::call_index(23)]
        #[pallet::weight(<T as Config>::WeightInfo::remove_authorized_attestor())]
        pub fn remove_authorized_attestor(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure!(
                AuthorizedAttestors::<T>::contains_key(chain_key, &attestor_id),
                Error::<T>::AttestorNotAuthorized
            );

            AuthorizedAttestors::<T>::remove(chain_key, &attestor_id);

            Self::deposit_event(Event::<T>::AuthorizedAttestorRemoved(
                chain_key,
                attestor_id,
            ));

            Ok(())
        }

        #[pallet::call_index(24)]
        #[pallet::weight(<T as Config>::WeightInfo::kick_active_attestor())]
        pub fn kick_active_attestor(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor_id: T::AccountId,
            unregister: bool,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            let attestor = Attestors::<T>::get(chain_key, &attestor_id)
                .ok_or(Error::<T>::AddressNotAttestor)?;

            Self::do_chill_attestor(chain_key, attestor_id.clone());

            // We also remove the attestor from the active attestor list if they are present
            ActiveAttestors::<T>::mutate(chain_key, |active_attestors| {
                if let Some(pos) = active_attestors.iter().position(|x| *x == attestor_id) {
                    active_attestors.swap_remove(pos);
                }
            });

            if unregister {
                // If unregister is true, also remove the attestor from the attestor list
                let stash = attestor.stash.clone();
                Self::remove_attestor_and_emit_event(chain_key, stash, attestor_id)?;
            }

            Ok(())
        }

        /// Force trigger an attestor election.
        ///
        /// A randomness of [0; 32] is used since randomness is not currently
        /// used in the election logic.
        #[pallet::call_index(25)]
        #[pallet::weight(<T as Config>::WeightInfo::force_election())]
        pub fn force_election(origin: OriginFor<T>, epoch: u64) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            Self::do_start_election(epoch, [0; 32])?;

            Self::deposit_event(Event::<T>::ForcedElection { epoch });

            Ok(())
        }

        /// Set the maximum catchup bound (in **blocks**) for a given chain.
        /// During catchup, continuity proofs will span at most this many
        /// blocks per attestation. Must be greater than zero. Takes effect
        /// at the next checkpoint.
        #[pallet::call_index(26)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_catchup())]
        pub fn set_max_catchup(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            max_catchup: u32,
        ) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            ensure! {
                max_catchup > 0,
                Error::<T>::InvalidMaxCatchup
            };

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );

            PendingMaxCatchup::<T>::set(chain_key, Some(max_catchup));

            Self::deposit_event(Event::<T>::PendingMaxCatchupSet(chain_key, max_catchup));

            Ok(())
        }

        /// Force apply all pending configuration updates immediately.
        ///
        /// This applies pending attestation intervals, target sample sizes,
        /// and max catchup values for all supported chains without waiting
        /// for the next epoch boundary.
        #[pallet::call_index(27)]
        #[pallet::weight(<T as Config>::WeightInfo::force_apply_updates())]
        pub fn force_apply_updates(origin: OriginFor<T>) -> DispatchResult {
            T::OperatorsOrigin::ensure_origin(origin)?;

            Self::apply_interval_updates();

            Self::deposit_event(Event::<T>::ForcedUpdatesApplied);

            Ok(())
        }
    }

    impl<T: Config> CheckpointProvider for Pallet<T> {
        fn get_checkpoint(chain_key: ChainKey, block_number: u64) -> Option<Digest> {
            Checkpoints::<T>::get(chain_key, block_number)
        }

        fn get_checkpoint_interval(chain_key: ChainKey) -> u64 {
            AttestationCheckpointInterval::<T>::get(chain_key).into()
        }

        fn get_last_checkpoint_number(chain_key: ChainKey) -> Option<u64> {
            LastCheckpoint::<T>::get(chain_key).map(|checkpoint| checkpoint.block_number)
        }
    }

    impl<T: Config> AttestationProvider<T::Hash, T::AccountId> for Pallet<T> {
        fn get_attestation(
            chain_key: ChainKey,
            digest: Digest,
        ) -> Option<SignedAttestation<T::Hash, T::AccountId>> {
            Attestations::<T>::get(chain_key, digest)
        }

        fn get_attestation_interval(chain_key: ChainKey) -> u64 {
            ChainAttestationInterval::<T>::get(chain_key)
        }
    }

    impl<T: Config> OnRegisterChainProvider for Pallet<T> {
        fn on_register_chain(
            chain_key: ChainKey,
            _chain_id: u64,
            _chain_name: Vec<u8>,
            target_sample_size: Option<u32>,
            chain_attestation_interval: Option<u64>,
            attestation_checkpoint_interval: Option<u32>,
            max_attestors: Option<u32>,
            max_invulnerables: Option<u32>,
            attestation_chain_genesis_block_number: Option<u64>,
            _encoding: ChainEncodingVersion,
        ) {
            TargetSampleSize::<T>::insert(
                chain_key,
                target_sample_size.unwrap_or(T::DefaultTargetSampleSize::get()),
            );

            Self::deposit_event(Event::<T>::TargetSampleSizeChanged(
                chain_key,
                target_sample_size.unwrap_or(T::DefaultTargetSampleSize::get()),
            ));

            ChainAttestationInterval::<T>::insert(
                chain_key,
                chain_attestation_interval.unwrap_or(T::DefaultAttestationInterval::get()),
            );

            Self::deposit_event(Event::<T>::AttestationIntervalChanged(
                chain_key,
                chain_attestation_interval.unwrap_or(T::DefaultAttestationInterval::get()),
            ));

            AttestationCheckpointInterval::<T>::insert(
                chain_key,
                attestation_checkpoint_interval
                    .unwrap_or(T::DefaultAttestationsPerCheckpoint::get()),
            );

            Self::deposit_event(Event::<T>::CheckpointIntervalChanged(
                chain_key,
                attestation_checkpoint_interval
                    .unwrap_or(T::DefaultAttestationsPerCheckpoint::get()),
            ));

            MaxAttestors::<T>::insert(
                chain_key,
                max_attestors.unwrap_or(T::MaxAttestationNodes::get()),
            );

            Self::deposit_event(Event::<T>::MaxAttestorsChanged(
                chain_key,
                max_attestors.unwrap_or(T::MaxAttestationNodes::get()),
            ));

            MaxInvulnerables::<T>::insert(
                chain_key,
                max_invulnerables.unwrap_or(T::MaxAttestationNodes::get()),
            );

            AttestationChainGenesisBlockNumber::<T>::insert(
                chain_key,
                attestation_chain_genesis_block_number
                    .unwrap_or(T::DefaultAttestationChainGenesisBlockNumber::get()),
            );

            Self::deposit_event(Event::<T>::AttestationChainGenesisBlockNumberSet(
                chain_key,
                attestation_chain_genesis_block_number
                    .unwrap_or(T::DefaultAttestationChainGenesisBlockNumber::get()),
            ));
        }
    }

    /// See the Polkadot SDK docs on [custom fees]
    ///
    /// [custom fees]: https://docs.polkadot.com/polkadot-protocol/parachain-basics/blocks-transactions-fees/fees/#custom-fees
    struct CommitAttestationWeight<T>(PhantomData<T>);

    impl<T> Default for CommitAttestationWeight<T> {
        fn default() -> Self {
            Self(PhantomData)
        }
    }

    impl<T: Config> WeighData<(&SignedAttestation<T::Hash, T::AccountId>,)>
        for CommitAttestationWeight<T>
    {
        fn weigh_data(&self, attestation: (&SignedAttestation<T::Hash, T::AccountId>,)) -> Weight {
            <T as Config>::WeightInfo::commit_attestation(
                attestation.0.continuity_proof.len() as u32,
                T::MaxAttestationNodes::get(),
            )
        }
    }

    impl<T: Config> ClassifyDispatch<(&SignedAttestation<T::Hash, T::AccountId>,)>
        for CommitAttestationWeight<T>
    {
        fn classify_dispatch(
            &self,

            _attestations: (&SignedAttestation<T::Hash, T::AccountId>,),
        ) -> DispatchClass {
            DispatchClass::Normal
        }
    }

    impl<T: Config> PaysFee<(&SignedAttestation<T::Hash, T::AccountId>,)>
        for CommitAttestationWeight<T>
    {
        /// Makes it so active attestors do not pay fees but regular accounts do
        fn pays_fee(&self, attestation: (&SignedAttestation<T::Hash, T::AccountId>,)) -> Pays {
            let chain_key = attestation.0.chain_key();

            let active_attestors = ActiveAttestors::<T>::get(chain_key)
                .into_iter()
                .collect::<BTreeSet<_>>();

            let is_attestor = attestation
                .0
                .attestors
                .iter()
                .all(|attestor| active_attestors.contains(attestor));

            if is_attestor {
                Pays::No
            } else {
                Pays::Yes
            }
        }
    }
}
