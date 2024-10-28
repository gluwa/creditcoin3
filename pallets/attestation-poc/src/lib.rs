#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

mod benchmarking;
#[cfg(test)]
mod tests;

mod asset;
mod impls;
mod ledger;

#[frame_support::pallet]
pub mod pallet {
    use crate::ledger::AttestorLedger;
    use attestor_primitives::{
        AttestationChainConfiguration, AttestationCheckpoint, Attestor, AttestorStatus,
        BlsPublicKey, BlsPublicKeyWrapper, BlsSignature, ChainAttestationIntervalType, ChainId,
        ChainKey, Digest, InherentError, SignedAttestation, INHERENT_IDENTIFIER,
    };
    use bls_signatures::{key::aggregate_public_keys, PublicKey, Serialize, Signature};
    use frame_support::{
        pallet_prelude::*,
        traits::{Currency, LockableCurrency, OnUnbalanced},
        transactional, Blake2_128Concat, Twox64Concat,
    };
    use frame_system::pallet_prelude::*;
    use log::debug;
    use parity_scale_codec::FullCodec;
    use sp_runtime::traits::SaturatedConversion;
    use sp_staking::StakingInterface;
    use sp_std::collections::vec_deque::VecDeque;
    use sp_std::{fmt::Debug, vec::Vec};
    use supported_chains_primitives::provider::SupportedChainsProvider;

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
        fn set_comittee_set_size() -> Weight;
        fn set_chain_attestation_interval() -> Weight;
        fn set_attestations_per_checkpoint() -> Weight;
        fn set_min_bond_requirement() -> Weight;
        fn chill() -> Weight;
        fn attest() -> Weight;
        fn set_payee() -> Weight;
        fn withdraw_unbonded() -> Weight;
        fn set_chain_reward() -> Weight;
        fn claim_rewards() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn attestors)]
    // Attestor storage maps a "Stash account" to an Attestor (controller)
    // Per supported chain
    pub type Attestors<T: Config> = StorageDoubleMap<
        _,
        Twox64Concat,
        ChainId,
        Blake2_128Concat,
        T::AccountId,
        Attestor<T::AccountId>,
    >;

    #[pallet::storage]
    #[pallet::getter(fn active_attestors)]
    // Active attestors are the ones that have been registered and are not in the chilling state
    pub type ActiveAttestors<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainId, Vec<T::AccountId>, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn invulnerables)]
    pub type Invulnerables<T: Config> =
        StorageDoubleMap<_, Twox64Concat, ChainId, Blake2_128Concat, T::AccountId, bool>;

    #[pallet::storage]
    #[pallet::getter(fn max_attestors)]
    pub type MaxAttestors<T: Config> = StorageMap<_, Blake2_128Concat, ChainId, u32, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn max_invulnerables)]
    pub type MaxInvulnerables<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainId, u32, ValueQuery>;

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
        ChainId,
        Blake2_128Concat,
        Digest,
        SignedAttestation<T::Hash, T::AccountId>,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn checkpoints)]
    pub type Checkpoints<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        ChainId,
        Blake2_128Concat,
        Digest,
        AttestationCheckpoint,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn checkpointing_queues)]
    pub type CheckpointingQueues<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainId, VecDeque<Digest>, ValueQuery, GetDefault>;

    #[pallet::storage]
    #[pallet::getter(fn last_attestation_digest)]
    pub type LastDigest<T: Config> = StorageMap<_, Blake2_128Concat, ChainId, Digest, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn comittee_set_size)]
    pub type ComitteeSetSize<T: Config> = StorageMap<_, Blake2_128Concat, ChainId, u32, ValueQuery>;

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
    #[pallet::getter(fn attestation_checkpoint_interval)]
    pub type AttestationCheckpointInterval<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainId,
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

    /// Map from all supported chain ids to the chain reward.
    ///
    /// This is used to store the reward for each chain.
    #[pallet::storage]
    #[pallet::getter(fn chain_reward)]
    pub type ChainReward<T: Config> =
        StorageMap<_, Blake2_128Concat, ChainId, BalanceOf<T>, OptionQuery>;

    /// Map from all the stash account id's to the reward that they have earned.
    ///
    /// This is used to store the reward for each stash account.
    #[pallet::storage]
    #[pallet::getter(fn accumulated_rewards)]
    pub type AccumulatedRewards<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, OptionQuery>;

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
                // Set the comittee set size for the chain
                ComitteeSetSize::<T>::insert(
                    chain_configuration.chain_id,
                    chain_configuration.comittee_set_size,
                );

                ChainAttestationInterval::<T>::insert(
                    chain_configuration.chain_id,
                    chain_configuration.attestation_interval,
                );
                AttestationCheckpointInterval::<T>::insert(
                    chain_configuration.chain_id,
                    chain_configuration.attestations_per_checkpoint,
                );
                ChainReward::<T>::insert(
                    chain_configuration.chain_id,
                    BalanceOf::<T>::saturated_from(chain_configuration.chain_reward),
                );

                MaxAttestors::<T>::insert(
                    chain_configuration.chain_id,
                    T::MaxAttestationNodes::get(),
                );

                MaxInvulnerables::<T>::insert(
                    chain_configuration.chain_id,
                    T::MaxAttestationNodes::get(),
                );

                for invulnerable in self.invulnerables.iter() {
                    Invulnerables::<T>::insert(
                        chain_configuration.chain_id,
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
        AttestorRegistered(ChainId, T::AccountId),
        AttestorUnregistered(ChainId, T::AccountId),
        /// Emitted when an invulnerable is properly registered with the attestation system
        InvulnerableRegistered(ChainId, T::AccountId),
        InvulnerableUnregistered(ChainId, T::AccountId),
        BlockAttested(ChainId, SignedAttestation<T::Hash, T::AccountId>, Digest),
        CheckpointReached(ChainId, AttestationCheckpoint),
        ComitteeSetSizeChanged(ChainId, u32),
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
        AttestorActivated(ChainId, T::AccountId),
        AttestorChilled(ChainId, T::AccountId),
        RewardPaid {
            chain_id: ChainId,
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        RewardClaimed {
            stash: T::AccountId,
            amount: BalanceOf<T>,
        },
        AttestorsElected {
            epoch: u64,
            chain_id: ChainId,
            attestors: Vec<T::AccountId>,
        },
        MinBondRequirementUpdated(BalanceOf<T>),
        ChainRewardUpdated(ChainId, BalanceOf<T>),

        /// Note a change in the attestation interval for a source chain. Also notes the
        /// block number of the latest attestation for that source chain at the time of
        /// the interval change.
        AttestationIntervalChanged(ChainId, ChainAttestationIntervalType, u64),
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
        // Did not find attestation which should exist. EX: Attestation with
        // digest matching LastDigest
        AttestationNotFound,
        // Could not retrieve SupportedChain info after already checking that
        // it exists.
        SupportedChainNotFound,
        // Tried to set attestation interval to an invalid value.
        InvalidAttestationInterval,
        // Tried to set attestations per checkpoint to an invalid value.
        InvalidAttestationsPerCheckpoint,
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

            ChainAttestationInterval::<T>::set(chain_key, chain_attestation_interval);

            let last_digest = Self::last_attestation_digest(chain_key);
            let last_block_number = if let Some(digest) = last_digest {
                Self::attestations(chain_key, digest)
                    .ok_or(Error::<T>::AttestationNotFound)?
                    .header_number()
            } else {
                0
            };

            // The error `SupportedChainNotFound` should never be hit, since the
            // problem it guards against is already covered by `ChainNotSupported`.
            // We still keep this error passing to avoid panic in case of future
            // code changes.
            let chain_id = T::SupportedChains::get_supported_chain(chain_key)
                .ok_or(Error::<T>::SupportedChainNotFound)?
                .chain_id;

            Self::deposit_event(Event::<T>::AttestationIntervalChanged(
                chain_id,
                chain_attestation_interval,
                last_block_number,
            ));
            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::set_comittee_set_size())]
        pub fn set_comittee_set_size(
            origin: OriginFor<T>,
            chain_id: ChainId,
            new_comittee_set_size: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ComitteeSetSize::<T>::insert(chain_id, new_comittee_set_size);

            Self::deposit_event(Event::<T>::ComitteeSetSizeChanged(
                chain_id,
                new_comittee_set_size,
            ));

            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::register_attestor())]
        pub fn register_attestor(
            origin: OriginFor<T>,
            chain_id: ChainId,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::try_insert_attestor_and_emit_event(chain_id, who, attestor_id)
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_attestor())]
        pub fn unregister_attestor(
            origin: OriginFor<T>,
            chain_id: ChainId,
            attestor_id: T::AccountId,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::remove_attestor_and_emit_event(chain_id, who, attestor_id)?;

            Ok(())
        }

        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_attestors())]
        pub fn set_max_attestors(
            origin: OriginFor<T>,
            chain_id: ChainId,
            new_max: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

            MaxAttestors::<T>::insert(chain_id, new_max);
            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::register_invulnerable())]
        pub fn register_invulnerable(
            origin: OriginFor<T>,
            chain_id: ChainId,
            attestor: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

            Self::try_insert_invulnerable_and_emit_event(chain_id, &attestor)
        }

        #[pallet::call_index(6)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_invulnerable())]
        pub fn unregister_invulnerable(
            origin: OriginFor<T>,
            chain_id: ChainId,
            attestor: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                Self::address_is_invulnerable(chain_id, &attestor),
                Error::<T>::AddressIsNotInvulnerable
            );

            Self::remove_invulnerable_and_emit_event(chain_id, attestor)
        }

        #[pallet::call_index(7)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_invulnerables())]
        pub fn set_max_invulnerables(
            origin: OriginFor<T>,
            chain_id: ChainId,
            new_max: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

            let count = Invulnerables::<T>::iter_prefix_values(chain_id)
                .collect::<Vec<_>>()
                .len() as u32;

            ensure!(
                new_max >= count,
                Error::<T>::MaxInvulnerablesCannotBeChanged
            );

            MaxInvulnerables::<T>::insert(chain_id, new_max);
            Ok(())
        }

        #[pallet::call_index(8)]
        #[pallet::weight(<T as Config>::WeightInfo::bootstrap_chain(attestation.attestors.len() as u32))]
        pub fn bootstrap_chain(
            origin: OriginFor<T>,
            chain_id: ChainId,
            attestation: SignedAttestation<T::Hash, T::AccountId>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_id),
                Error::<T>::ChainNotSupported
            );

            let previous_digest = Self::last_digest(chain_id);

            // Store the attestation
            let digest = attestation.digest();
            let header_number = attestation.header_number();
            Attestations::<T>::insert(chain_id, digest, &attestation);

            // Update last digest
            LastDigest::<T>::set(chain_id, Some(digest));

            Self::deposit_event(Event::<T>::BlockAttested(
                chain_id,
                attestation.clone(),
                digest,
            ));

            match previous_digest {
                None => {
                    // Very first attestation should have a corresponding checkpoint
                    // even though it doesn't condense any prior attestations.
                    let checkpoint = AttestationCheckpoint {
                        block_number: header_number,
                        digest,
                    };

                    Self::deposit_event(Event::<T>::CheckpointReached(
                        chain_id,
                        checkpoint.clone(),
                    ));

                    Checkpoints::<T>::insert(chain_id, checkpoint.digest, checkpoint);
                }
                Some(_prev_digest) => {
                    // Add to checkpointing queue
                    let mut queue = CheckpointingQueues::<T>::get(chain_id);
                    queue.push_back(digest);

                    // Make checkpoint if necessary.
                    // The extrinsic didn't fail even if checkpointing failed. We want
                    // to keep the new attestation rather than removing it from storage
                    // via extrinsic rollback in the case of checkpointing failure.
                    if let Err(e) = Self::try_make_checkpoint(&mut queue, chain_id) {
                        log::error!("Error: {:?}", e);
                    }
                    CheckpointingQueues::<T>::insert(chain_id, queue);
                }
            }

            Ok(())
        }

        #[pallet::call_index(9)]
        #[pallet::weight((<T as Config>::WeightInfo::commit_attestation(attestation.attestors.len() as u32), DispatchClass::Mandatory))]
        pub fn commit_attestation(
            origin: OriginFor<T>,
            attestation: SignedAttestation<T::Hash, T::AccountId>,
        ) -> DispatchResult {
            ensure_none(origin)?;

            let chain_id = attestation.chain_id();
            ensure!(
                T::SupportedChains::is_chain_supported(chain_id),
                Error::<T>::ChainNotSupported
            );

            ensure!(
                !Attestations::<T>::contains_key(chain_id, attestation.digest()),
                Error::<T>::AttestationExists
            );

            ensure!(
                Self::validate_attestation(&attestation).is_ok(),
                Error::<T>::InvalidAttestation
            );

            let previous_digest = Self::last_digest(chain_id);
            ensure!(
                previous_digest == attestation.attestation.prev_digest,
                Error::<T>::InvalidAttestation
            );

            if let Some(previous_digest) = previous_digest {
                let previous_attestation = Attestations::<T>::get(chain_id, previous_digest)
                    .ok_or(Error::<T>::NoPreviousDigest)?;

                let interval = ChainAttestationInterval::<T>::get(chain_id);
                let prev_block_number = previous_attestation.attestation.header_number;

                debug!(
                    "Checking if block number is at the interval, expected: {:?}, got: {:?}",
                    prev_block_number + interval,
                    attestation.attestation.header_number
                );

                if attestation.attestation.header_number != prev_block_number + interval {
                    debug!(
                        "Block number is not at the interval, expected: {:?}, got: {:?}",
                        prev_block_number + interval,
                        attestation.attestation.header_number
                    );
                    return Err(Error::<T>::InvalidAttestation.into());
                }
            }

            // Store the attestation
            let digest = attestation.digest();
            let header_number = attestation.header_number();
            Attestations::<T>::insert(chain_id, digest, &attestation);

            // Update last digest
            LastDigest::<T>::set(chain_id, Some(digest));

            // Pay out attestation rewards
            Self::payout_attestors(chain_id, &attestation.attestors)?;

            // Emit event
            Self::deposit_event(Event::<T>::BlockAttested(chain_id, attestation, digest));

            match previous_digest {
                None => {
                    // Very first attestation should have a corresponding checkpoint
                    // even though it doesn't condense any prior attestations.
                    let checkpoint = AttestationCheckpoint {
                        block_number: header_number,
                        digest,
                    };

                    Self::deposit_event(Event::<T>::CheckpointReached(
                        chain_id,
                        checkpoint.clone(),
                    ));

                    Checkpoints::<T>::insert(chain_id, checkpoint.digest, checkpoint);
                }
                Some(_prev_digest) => {
                    // Add to checkpointing queue
                    let mut queue = CheckpointingQueues::<T>::get(chain_id);
                    queue.push_back(digest);

                    // Make checkpoint if necessary.
                    // The extrinsic didn't fail even if checkpointing failed. We want
                    // to keep the new attestation rather than removing it from storage
                    // via extrinsic rollback in the case of checkpointing failure.
                    if let Err(e) = Self::try_make_checkpoint(&mut queue, chain_id) {
                        log::error!("Error: {:?}", e);
                    }
                    CheckpointingQueues::<T>::insert(chain_id, queue);
                }
            }

            Ok(())
        }

        #[pallet::call_index(10)]
        #[pallet::weight(<T as Config>::WeightInfo::set_attestations_per_checkpoint())]
        pub fn set_attestations_per_checkpoint(
            origin: OriginFor<T>,
            chain_id: ChainId,
            attestations_per_checkpoint: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure! {
                attestations_per_checkpoint > 0,
                Error::<T>::InvalidAttestationsPerCheckpoint
            };

            ensure!(
                T::SupportedChains::is_chain_supported(chain_id),
                Error::<T>::ChainNotSupported
            );

            AttestationCheckpointInterval::<T>::set(chain_id, attestations_per_checkpoint);
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
            chain_id: ChainId,
            reward: BalanceOf<T>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_id),
                Error::<T>::ChainNotSupported
            );

            ChainReward::<T>::insert(chain_id, reward);

            Self::deposit_event(Event::<T>::ChainRewardUpdated(chain_id, reward));

            Ok(())
        }

        #[pallet::call_index(13)]
        #[pallet::weight(<T as Config>::WeightInfo::attest())]
        pub fn attest(
            origin: OriginFor<T>,
            chain_id: ChainId,
            bls_public_key: BlsPublicKey,
            proof_of_possession: BlsSignature,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            Self::start_attesting(chain_id, who, bls_public_key, proof_of_possession)?;

            Ok(())
        }

        #[pallet::call_index(14)]
        #[pallet::weight(<T as Config>::WeightInfo::chill())]
        pub fn chill(origin: OriginFor<T>, chain_id: ChainId) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let mut attestor =
                Attestors::<T>::get(chain_id, &who).ok_or(Error::<T>::AddressNotAttestor)?;

            attestor.status = AttestorStatus::Idle;
            Attestors::<T>::insert(chain_id, &who, attestor);

            Self::deposit_event(Event::<T>::AttestorChilled(chain_id, who));

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
            let inherent_data = data
                .get_data::<SignedAttestation<T::Hash, T::AccountId>>(&INHERENT_IDENTIFIER)
                .expect("Attestation inherent data not correctly encoded");

            // Check if atleast the attestation was not already submitted
            if let Some(attestation) = inherent_data {
                if let Some(digest) = LastDigest::<T>::get(attestation.attestation.chain_id) {
                    if digest == attestation.digest() {
                        log::error!("Attestation with digest: {:?} is duplicate", digest);
                        return None;
                    }
                };
                if !T::SupportedChains::is_chain_supported(attestation.chain_id()) {
                    log::error!(
                        "Chain with id: {:?} is not supported",
                        attestation.chain_id()
                    );
                    return None;
                }

                Some(Call::commit_attestation { attestation })
            } else {
                None
            }
        }

        fn check_inherent(
            call: &Self::Call,
            _data: &InherentData,
        ) -> sp_std::result::Result<(), Self::Error> {
            match call {
                Call::commit_attestation { attestation } => {
                    Pallet::<T>::check_duplicate(attestation)?;
                    let agg_signature = Pallet::<T>::extract_agg_signature(&attestation.signature)?;
                    let attestor_public_keys = Pallet::<T>::gather_attestor_public_keys(
                        attestation.chain_id(),
                        &attestation.attestors,
                    )?;
                    let aggregated_public_key = aggregate_public_keys(&attestor_public_keys[..])
                        .map_err(|_| {
                            log::error!("Failed to aggregate public keys");
                            InherentError::NotValid
                        })?;

                    let message = &attestation.attestation.serialize()[..];

                    Pallet::<T>::verify_agg_signature(
                        &agg_signature,
                        message,
                        aggregated_public_key,
                    )?;

                    log::info!("Attestation signature is valid");
                    Ok(())
                }
                _ => Err(InherentError::NotValid),
            }
        }

        fn is_inherent(call: &Self::Call) -> bool {
            matches!(call, Call::commit_attestation { .. })
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn working_set_size(chain_id: ChainId) -> u32 {
            ActiveAttestors::<T>::get(chain_id).len() as u32
        }

        pub fn is_attestor(chain_id: ChainId, address: &T::AccountId) -> bool {
            let active_attestors = ActiveAttestors::<T>::get(chain_id);
            active_attestors.contains(address)
        }

        pub fn attestor_status(
            chain_id: ChainId,
            address: &T::AccountId,
        ) -> Option<AttestorStatus> {
            Attestors::<T>::get(chain_id, address).map(|attestor| attestor.status)
        }

        pub fn address_is_not_attestor(chain_id: ChainId, address: &T::AccountId) -> bool {
            !Self::is_attestor(chain_id, address)
        }

        pub fn attestor_is_registered(chain_id: ChainId, address: &T::AccountId) -> bool {
            Attestors::<T>::contains_key(chain_id, address)
        }

        pub fn last_digest(chain_id: ChainId) -> Option<Digest> {
            LastDigest::<T>::get(chain_id)
        }

        pub fn contains_digest(chain_id: ChainId, digest: Digest) -> bool {
            Attestations::<T>::contains_key(chain_id, digest)
        }

        pub fn attestor_bls_pubkey(
            chain_id: ChainId,
            address: &T::AccountId,
        ) -> Option<BlsPublicKey> {
            let pk = Attestors::<T>::get(chain_id, address).map(|attestor| attestor.bls_public_key);
            match pk {
                Some(pk) => pk,
                None => None,
            }
        }

        pub fn attestor_list_has_space(chain_id: ChainId) -> bool {
            let count = Attestors::<T>::iter_prefix_values(chain_id)
                .collect::<Vec<_>>()
                .len() as u32;
            count < MaxAttestors::<T>::get(chain_id)
        }

        pub fn get(
            chain_id: ChainId,
            digest: Digest,
        ) -> Option<SignedAttestation<T::Hash, T::AccountId>> {
            Attestations::<T>::get(chain_id, digest)
        }

        fn vulnerable_list_has_space(chain_id: ChainId) -> bool {
            let count = Invulnerables::<T>::iter_prefix_values(chain_id)
                .collect::<Vec<_>>()
                .len() as u32;
            count < MaxInvulnerables::<T>::get(chain_id)
        }

        /// Insert address as attestor & invulnerable
        fn try_insert_invulnerable_and_emit_event(
            chain_id: ChainId,
            address: &T::AccountId,
        ) -> DispatchResult {
            ensure!(
                Self::vulnerable_list_has_space(chain_id),
                Error::<T>::InvulnerableListFull
            );

            Invulnerables::<T>::insert(chain_id, address, true);
            Self::deposit_event(Event::<T>::InvulnerableRegistered(
                chain_id,
                address.clone(),
            ));
            Ok(())
        }

        fn address_is_invulnerable(chain_id: ChainId, address: &T::AccountId) -> bool {
            Invulnerables::<T>::contains_key(chain_id, address)
        }

        // Remove address as invulnerable and attestor
        fn remove_invulnerable_and_emit_event(
            chain_id: ChainId,
            address: T::AccountId,
        ) -> DispatchResult {
            // Remove from invulnerables
            Invulnerables::<T>::remove(chain_id, &address);
            Self::deposit_event(Event::<T>::InvulnerableUnregistered(
                chain_id,
                address.clone(),
            ));

            Ok(())
        }

        fn validate_attestation(
            attestation: &SignedAttestation<T::Hash, T::AccountId>,
        ) -> Result<(), InherentError> {
            Self::check_duplicate(attestation)?;
            let agg_signature = Self::extract_agg_signature(&attestation.signature)?;
            let attestor_public_keys =
                Self::gather_attestor_public_keys(attestation.chain_id(), &attestation.attestors)?;
            let aggregated_public_key =
                aggregate_public_keys(&attestor_public_keys[..]).map_err(|_| {
                    log::error!("Failed to aggregate public keys");
                    InherentError::NotValid
                })?;

            let message = &attestation.attestation.serialize()[..];

            Self::verify_agg_signature(&agg_signature, message, aggregated_public_key)?;

            log::info!("Attestation signature is valid");

            Ok(())
        }

        // When current checkpoint interval is completed by the commitment of its final attestation,
        // then the prior checkpoint interval is considered "stabilized". We condense all the
        // attestations for that prior interval into a single checkpoint.
        #[transactional]
        fn try_make_checkpoint(queue: &mut VecDeque<Digest>, chain_id: ChainId) -> DispatchResult {
            let num_to_condense = Self::attestation_checkpoint_interval(chain_id);
            // Only move forward if two full checkpoints of attestations are committed.
            if queue.len() < (num_to_condense * 2) as usize {
                return Ok(());
            }

            // Because checkpointing queue storage is written to after this function
            // returns, it isn't covered by the #[transactional] macro and must be manually
            // rolled back.
            let mut checkpointing_rollback: Vec<Digest> = Vec::new();
            for i in 0..num_to_condense {
                let to_be_removed: Digest = match queue.pop_front() {
                    Some(digest) => digest,
                    None => {
                        for digest in checkpointing_rollback {
                            queue.push_front(digest);
                        }
                        return Err(Error::<T>::CheckpointCreationError.into());
                    }
                };
                checkpointing_rollback.push(to_be_removed);

                // Until then, removing attestations from storage breaks proving.
                let removed = match Attestations::<T>::take(chain_id, to_be_removed) {
                    Some(attestation) => attestation,
                    None => {
                        for digest in checkpointing_rollback {
                            queue.push_front(digest);
                        }
                        return Err(Error::<T>::CheckpointCreationError.into());
                    }
                };

                if i == num_to_condense - 1 {
                    let checkpoint = AttestationCheckpoint {
                        block_number: removed.header_number(),
                        digest: removed.digest(),
                    };

                    Self::deposit_event(Event::<T>::CheckpointReached(
                        chain_id,
                        checkpoint.clone(),
                    ));

                    Checkpoints::<T>::insert(chain_id, checkpoint.digest, checkpoint);
                }
            }

            Ok(())
        }

        #[cfg(test)]
        pub(crate) fn break_checkpointing() {
            let test_chain_id = 1;
            CheckpointingQueues::<T>::mutate(test_chain_id, |queue| {
                queue.push_back([0u8; 32].into());
            })
        }
    }
    // helper functions for checking inherent data
    impl<T: Config> Pallet<T> {
        fn check_duplicate(
            attestation: &SignedAttestation<T::Hash, T::AccountId>,
        ) -> Result<(), InherentError> {
            if let Some(digest) = LastDigest::<T>::get(attestation.attestation.chain_id) {
                if digest == attestation.attestation.digest() {
                    log::error!("Attestation with digest: {:?} is duplicate", digest);
                    return Err(InherentError::Duplicate(digest));
                }
            }
            Ok(())
        }

        fn extract_agg_signature(signature: &[u8]) -> Result<Signature, InherentError> {
            Signature::from_bytes(signature).map_err(|_| {
                log::error!("Failed to aggregate BLS signature");
                InherentError::NotValid
            })
        }

        fn gather_attestor_public_keys(
            chain_id: ChainId,
            attestors: &[T::AccountId],
        ) -> Result<Vec<PublicKey>, InherentError> {
            attestors
                .iter()
                .map(|attestor| {
                    let active_attestors = ActiveAttestors::<T>::get(chain_id);
                    let contains = active_attestors.contains(attestor);

                    if contains {
                        let attestor =
                            Attestors::<T>::get(chain_id, attestor).ok_or_else(|| {
                                log::error!("Attestor {:?} not found", attestor);
                                InherentError::InvalidAttestorFound
                            })?;
                        match attestor.bls_public_key {
                            Some(key) => PublicKey::from_bytes(&key[..]).map_err(|_| {
                                log::error!("Invalid BLS key for attestor {:?}", attestor);
                                InherentError::AttestorWithInvalidPublicKey
                            }),
                            None => {
                                log::error!("No BLS key for attestor {:?}", attestor);
                                Err(InherentError::AttestorWithInvalidPublicKey)
                            }
                        }
                    } else {
                        log::error!("Attestor {:?} is not active", attestor);
                        Err(InherentError::AttestorNotActive)
                    }
                })
                .collect()
        }

        fn verify_agg_signature(
            agg_signature: &Signature,
            message: &[u8],
            agg_public_key: PublicKey,
        ) -> Result<(), InherentError> {
            if !bls_signatures::verify_agg_message(agg_signature, message, agg_public_key) {
                log::error!("Aggregated signature is invalid");
                return Err(InherentError::NotValid);
            }
            Ok(())
        }
    }
}
