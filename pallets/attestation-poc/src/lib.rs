#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

mod asset;
mod impls;
mod ledger;

#[frame_support::pallet]
pub mod pallet {
    use crate::ledger::AttestorLedger;
    use attestor_primitives::{
        provider::{AttestationProvider, CheckpointProvider},
        AttestationChainConfiguration, AttestationCheckpoint, Attestor, BlsPublicKey,
        BlsPublicKeyWrapper, BlsSignature, ChainAttestationIntervalType, ChainKey, Digest,
        InherentError, SignedAttestation, INHERENT_IDENTIFIER,
    };
    use frame_support::{
        pallet_prelude::{OptionQuery, *},
        traits::{Currency, LockableCurrency, OnUnbalanced},
        Blake2_128Concat, Twox64Concat,
    };
    use frame_system::pallet_prelude::*;
    use parity_scale_codec::FullCodec;
    use sp_runtime::traits::SaturatedConversion;
    use sp_staking::StakingInterface;
    use sp_std::collections::vec_deque::VecDeque;
    use sp_std::{fmt::Debug, vec::Vec};
    use supported_chains_primitives::provider::SupportedChainsProvider;

    pub const MAX_CHECKPOINTS_CLEARED_PER_BLOCK: u8 = 40;

    /// The balance type of this pallet.
    pub type BalanceOf<T> = <T as Config>::CurrencyBalance;
    pub type PositiveImbalanceOf<T> = <<T as Config>::Currency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::PositiveImbalance;

    /// A destination account for payment.
    #[derive(PartialEq, Eq, Copy, Clone, Encode, Decode, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum RewardDestination<AccountId> {
        /// Pay into the stash account.
        Stash,
        /// Pay into a specified account.
        Account(AccountId),
        /// Receive no reward.
        None,
    }

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
        /// `From<u64>`.
        type CurrencyBalance: sp_runtime::traits::AtLeast32BitUnsigned
            + FullCodec
            + Copy
            + MaybeSerializeDeserialize
            + core::fmt::Debug
            + Default
            + From<u64>
            + TypeInfo
            + MaxEncodedLen;
        #[pallet::constant]
        type DefaultAttestationsPerCheckpoint: Get<u32>;
        #[pallet::constant]
        type DefaultAttestationInterval: Get<ChainAttestationIntervalType>;
        #[pallet::constant]
        type DefaultTargetSampleSize: Get<u32>;
        #[pallet::constant]
        type MaxAttestationNodes: Get<u32>;
        // TODO: Make this useful
        #[pallet::constant]
        type CommittmentInterval: Get<u64>;
        #[pallet::constant]
        type DefaultMinBondRequirement: Get<u64>;
        #[pallet::constant]
        type MaxUnlockingChunks: Get<u32>;
        /// Number of eras that staked funds must remain bonded for.
        #[pallet::constant]
        type BondingDuration: Get<u32>;
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
    }

    pub trait WeightInfo {
        fn register_attestor() -> Weight;
        fn unregister_attestor() -> Weight;
        fn set_max_attestors() -> Weight;
        fn register_invulnerable() -> Weight;
        fn unregister_invulnerable() -> Weight;
        fn set_max_invulnerables() -> Weight;
        fn bootstrap_chain(a: u32) -> Weight;
        fn commit_attestation(a: u32) -> Weight;
        fn set_target_sample_size() -> Weight;
        fn set_chain_attestation_interval() -> Weight;
        fn set_attestations_per_checkpoint() -> Weight;
        fn set_min_bond_requirement() -> Weight;
        fn chill() -> Weight;
        fn attest() -> Weight;
        fn set_payee() -> Weight;
        fn withdraw_unbonded() -> Weight;
        fn set_chain_reward() -> Weight;
        fn claim_rewards() -> Weight;
        fn on_initialize(a: u32) -> Weight;
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
        StorageDoubleMap<_, Blake2_128Concat, ChainKey, Blake2_128Concat, Digest, u64, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn last_checkpoint)]
    pub type LastCheckpoint<T> = StorageMap<_, Blake2_128Concat, ChainKey, AttestationCheckpoint>;

    #[pallet::storage]
    #[pallet::getter(fn checkpointing_queues)]
    pub type CheckpointingQueues<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, VecDeque<Digest>, ValueQuery, GetDefault>;

    #[pallet::storage]
    #[pallet::getter(fn last_attestation_digest)]
    pub type LastDigest<T: Config> = StorageMap<_, Blake2_128Concat, ChainKey, Digest, OptionQuery>;

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

    #[pallet::storage]
    #[pallet::getter(fn min_bond_requirement)]
    pub type MinBondRequirement<T: Config> =
        StorageValue<_, BalanceOf<T>, ValueQuery, DefaultMinBondRequirement<T>>;

    #[pallet::type_value]
    pub fn DefaultMinBondRequirement<T: Config>() -> BalanceOf<T> {
        T::DefaultMinBondRequirement::get().into()
    }

    /// Where the reward payment should be made. Keyed by stash.
    ///
    /// TWOX-NOTE: SAFE since `AccountId` is a secure hash.
    #[pallet::storage]
    pub type Payee<T: Config> =
        StorageMap<_, Twox64Concat, T::AccountId, RewardDestination<T::AccountId>, OptionQuery>;

    /// Map from all (unlocked) "controller" accounts to info regarding staking.
    ///
    /// Note: All the reads and mutations to this storage *MUST* be done through the methods exposed
    /// by [`AttestorLedger`] to ensure data and lock consistency.
    #[pallet::storage]
    #[pallet::getter(fn ledger)]
    pub type Ledger<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, AttestorLedger<T>>;

    /// Map from all supported chain keys to the chain rewards.
    ///
    /// This is used to store the reward for each chain.
    #[pallet::storage]
    #[pallet::getter(fn chain_reward)]
    pub type ChainReward<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, BalanceOf<T>, OptionQuery>;

    /// Map from all the stash account id's to the reward that they have earned.
    ///
    /// This is used to store the reward for each stash account.
    #[pallet::storage]
    #[pallet::getter(fn accumulated_rewards)]
    pub type AccumulatedRewards<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, OptionQuery>;

    /// Progress markers for removing the checkpoints associated with source chains that are
    /// no longer supported. Maps from a chain_key to a cursor representing the point up to which
    /// that chain's checkpoints have been removed.
    #[pallet::storage]
    #[pallet::getter(fn checkpoint_clearing_cursors)]
    pub type CheckpointClearingCursors<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainKey, Vec<u8>, OptionQuery>;

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
                ChainReward::<T>::insert(
                    chain_configuration.chain_key,
                    BalanceOf::<T>::saturated_from(chain_configuration.chain_reward),
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
        BlockAttested(ChainKey, SignedAttestation<T::Hash, T::AccountId>, Digest),
        CheckpointReached(ChainKey, AttestationCheckpoint),
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
        RewardPaid {
            chain_key: ChainKey,
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        RewardClaimed {
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        AttestorsElected {
            epoch: u64,
            chain_key: ChainKey,
            attestors: Vec<T::AccountId>,
        },
        MinBondRequirementUpdated(BalanceOf<T>),
        ChainRewardUpdated(ChainKey, BalanceOf<T>),

        /// Note a change in the attestation interval for a source chain. Also notes the
        /// block number of the latest attestation for that source chain at the time of
        /// the interval change.
        AttestationIntervalChanged(ChainKey, ChainAttestationIntervalType),
        PendingAttestationIntervalSet(ChainKey, ChainAttestationIntervalType),
        /// Signals that checkpoints were cleared for a chain that is no longer supported.
        /// A fixed number of checkpoints will be cleared per block until none remain.
        CheckpointsCleared(ChainKey),
        CheckpointIntervalChanged(ChainKey, u32),
        /// A source chain was removed via pallet supported chains. Associated storage
        /// in pallet attestation was cleaned up.
        ClearedStorageForRemovedChain(ChainKey),
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
        // Chain reward not found
        ChainRewardNotFound,
        // No rewards to claim
        NoRewards,
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
        // Tried to set committee set size to an invalid value.
        InvalidTargetSampleSize,
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

                Self::deposit_event(Event::<T>::CheckpointsCleared(chain_key));

                // Cleared checkpoints for 1 chain
                <T as Config>::WeightInfo::on_initialize(1)
            } else {
                // Cleared checkpoints for 0 chains
                <T as Config>::WeightInfo::on_initialize(0)
            }
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::set_chain_attestation_interval())]
        pub fn set_chain_attestation_interval(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            chain_attestation_interval: ChainAttestationIntervalType,
        ) -> DispatchResult {
            ensure_root(origin)?;

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
            ensure_root(origin)?;

            ensure! {
                new_target_sample_size > 0,
                Error::<T>::InvalidTargetSampleSize
            };

            TargetSampleSize::<T>::insert(chain_key, new_target_sample_size);

            Self::deposit_event(Event::<T>::TargetSampleSizeChanged(
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
            ensure_root(origin)?;

            MaxAttestors::<T>::insert(chain_key, new_max);
            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::register_invulnerable())]
        pub fn register_invulnerable(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

            Self::try_insert_invulnerable_and_emit_event(chain_key, &attestor)
        }

        #[pallet::call_index(6)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_invulnerable())]
        pub fn unregister_invulnerable(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestor: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

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
            ensure_root(origin)?;

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
            ensure_root(origin)?;

            Self::do_bootstrap_chain(attestation)
        }

        #[pallet::call_index(9)]
        #[pallet::weight((<T as Config>::WeightInfo::commit_attestation(attestations.len() as u32), DispatchClass::Mandatory))]
        pub fn commit_attestation(
            origin: OriginFor<T>,
            attestations: BoundedVec<
                SignedAttestation<T::Hash, T::AccountId>,
                T::MaxAttestationsPerBlock,
            >,
        ) -> DispatchResult {
            ensure_none(origin)?;

            for attestation in attestations.into_iter() {
                Self::do_commit_attestation(attestation)?;
            }

            Ok(())
        }

        #[pallet::call_index(10)]
        #[pallet::weight(<T as Config>::WeightInfo::set_attestations_per_checkpoint())]
        pub fn set_attestations_per_checkpoint(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            attestations_per_checkpoint: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

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
            min_bond_requirement: BalanceOf<T>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            MinBondRequirement::<T>::set(min_bond_requirement);

            Self::deposit_event(Event::<T>::MinBondRequirementUpdated(min_bond_requirement));

            Ok(())
        }

        #[pallet::call_index(12)]
        #[pallet::weight(<T as Config>::WeightInfo::set_chain_reward())]
        pub fn set_chain_reward(
            origin: OriginFor<T>,
            chain_key: ChainKey,
            reward: BalanceOf<T>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_key),
                Error::<T>::ChainNotSupported
            );

            ChainReward::<T>::insert(chain_key, reward);

            Self::deposit_event(Event::<T>::ChainRewardUpdated(chain_key, reward));

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

            Self::do_chill_attestor(chain_key, attestor_id, attestor);

            Ok(())
        }

        #[pallet::call_index(15)]
        #[pallet::weight(<T as Config>::WeightInfo::set_payee())]
        pub fn set_payee(
            origin: OriginFor<T>,
            payee: RewardDestination<T::AccountId>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let ledger = Self::ledger(&who).ok_or(Error::<T>::NotStash)?;
            ledger.set_payee(payee)?;

            Ok(())
        }

        #[pallet::call_index(16)]
        #[pallet::weight(<T as Config>::WeightInfo::withdraw_unbonded())]
        pub fn withdraw_unbonded(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::do_withdraw_unbonded(&who)?;

            Ok(())
        }

        #[pallet::call_index(17)]
        #[pallet::weight(<T as Config>::WeightInfo::claim_rewards())]
        pub fn claim_rewards(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::do_claim_rewards(who)?;

            Ok(())
        }
    }

    #[pallet::inherent]
    impl<T: Config> ProvideInherent for Pallet<T> {
        type Call = Call<T>;
        type Error = InherentError;
        const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

        fn create_inherent(data: &InherentData) -> Option<Self::Call> {
            let inherent_data =
                data.get_data::<BoundedVec<
                    SignedAttestation<T::Hash, T::AccountId>,
                    T::MaxAttestationsPerBlock,
                >>(&INHERENT_IDENTIFIER)
                    .expect("Attestation inherent data not correctly encoded");

            // Check if at least one attestation can be submitted
            if let Some(attestations) = inherent_data {
                let valid_attestations: Vec<_> = attestations
                    .into_iter()
                    .filter(|attestation| {
                        // Check if the attestation is valid, if not filter it out
                        if Self::validate_attestation(attestation.chain_key(), attestation).is_err()
                        {
                            log::info!(
                                "📝 Attestation with digest {:?} is invalid",
                                attestation.digest()
                            );
                            false
                        } else {
                            log::info!(
                                "📝 Attestation with digest {:?} is valid ✅",
                                attestation.digest()
                            );
                            true
                        }
                    })
                    .collect();

                if valid_attestations.is_empty() {
                    return None;
                }

                Some(Call::commit_attestation {
                    attestations: valid_attestations.try_into().unwrap(),
                })
            } else {
                None
            }
        }

        fn check_inherent(
            call: &Self::Call,
            _data: &InherentData,
        ) -> sp_std::result::Result<(), Self::Error> {
            match call {
                Call::commit_attestation { .. } => Ok(()),
                _ => Err(InherentError::NotValid),
            }
        }

        fn is_inherent(call: &Self::Call) -> bool {
            matches!(call, Call::commit_attestation { .. })
        }
    }

    impl<T: Config> CheckpointProvider for Pallet<T> {
        fn get_checkpoint(chain_key: ChainKey, digest: Digest) -> Option<AttestationCheckpoint> {
            Checkpoints::<T>::get(chain_key, digest)
        }
    }

    impl<T: Config> AttestationProvider<T::Hash, T::AccountId> for Pallet<T> {
        fn get_attestation(
            chain_key: ChainKey,
            digest: Digest,
        ) -> Option<SignedAttestation<T::Hash, T::AccountId>> {
            Attestations::<T>::get(chain_key, digest)
        }
    }
}
