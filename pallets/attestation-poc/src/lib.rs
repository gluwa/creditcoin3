#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

mod benchmarking;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    use attestor_primitives::{
        AttestationCheckpoint, BlsPublicKey, BlsPublicKeyWrapper, BlsSignature, ChainId, Digest,
        InherentError, SignedAttestation, INHERENT_IDENTIFIER,
    };
    use bls_signatures::{key::aggregate_public_keys, PublicKey, Serialize, Signature};
    use frame_support::pallet_prelude::{
        CountedStorageMap, DispatchResult, OptionQuery, ValueQuery,
    };
    use frame_support::{pallet_prelude::*, transactional, Blake2_128Concat};
    use frame_system::pallet_prelude::*;
    use log::debug;
    use parity_scale_codec::FullCodec;
    use sp_std::collections::vec_deque::VecDeque;
    use sp_std::{fmt::Debug, vec::Vec};
    use supported_chains_primitives::provider::SupportedChainsProvider;

    pub type ChainAttestationIntervalType = u64;

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        #[pallet::constant]
        type DefaultAttestationsPerCheckpoint: Get<u32>;
        #[pallet::constant]
        type DefaultAttestationInterval: Get<ChainAttestationIntervalType>;
        #[pallet::constant]
        type MaxAttestationNodes: Get<u32>;
        // TODO: Make this useful
        #[pallet::constant]
        type CommittmentInterval: Get<u64>;

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
    }

    #[pallet::storage]
    #[pallet::getter(fn attestors)]
    pub type Attestors<T: Config> = CountedStorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        Value = BlsPublicKey,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn invulnerables)]
    pub type Invlunerables<T: Config> = CountedStorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        // Value can be ignored
        Value = bool,
        QueryKind = ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn max_attestors)]
    pub type MaxAttestors<T: Config> = StorageValue<_, u32, ValueQuery, MaxAttestorsDefault<T>>;

    #[pallet::storage]
    #[pallet::getter(fn max_invulnerables)]
    pub type MaxInvulnerables<T: Config> =
        StorageValue<_, u32, ValueQuery, MaxInvulernablesDefault<T>>;

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
    #[pallet::getter(fn last_attesation_digest)]
    pub type LastDigest<T: Config> = StorageMap<_, Blake2_128Concat, ChainId, Digest, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn comittee_set_size)]
    pub type ComitteeSetSize<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn chain_attestation_interval)]
    pub type ChainAttestationInterval<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        ChainId,
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

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub comittee_set_size: u32,
        pub invulnerables: Vec<(T::AccountId, BlsPublicKeyWrapper)>,
        pub attestation_chains_interval: Vec<(ChainId, ChainAttestationIntervalType)>,
        pub attestations_per_checkpoint: Vec<(ChainId, u32)>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let comittee_set_size = &self.comittee_set_size;
            ComitteeSetSize::<T>::put(comittee_set_size);

            let invulnerables = &self.invulnerables;
            for invulnerable in invulnerables.iter() {
                Invlunerables::<T>::insert(invulnerable.0.clone(), true);
                Attestors::<T>::insert(invulnerable.0.clone(), invulnerable.1 .0);
            }

            for (chain_id, chain_attestation_interval) in
                self.attestation_chains_interval.clone().into_iter()
            {
                ChainAttestationInterval::<T>::insert(chain_id, chain_attestation_interval);
            }

            for (chain_id, att_per_check) in self.attestations_per_checkpoint.clone().into_iter() {
                AttestationCheckpointInterval::<T>::insert(chain_id, att_per_check);
            }
        }
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Emitted when an attestor is properly registered with the attestation system
        AttestorRegistered(T::AccountId),

        AttestorUnregistered(T::AccountId),

        /// Emitted when an invulnerable is properly registered with the attestation system
        InvulnerableRegistered(T::AccountId),

        InvulnerableUnregistered(T::AccountId),

        ChainBootstrapped(ChainId, SignedAttestation<T::Hash, T::AccountId>),

        BlockAttested(ChainId, SignedAttestation<T::Hash, T::AccountId>),

        CheckpointReached(ChainId, AttestationCheckpoint),

        ComitteeSetSizeChanged(u32),
    }

    #[pallet::error]
    pub enum Error<T> {
        //// If we cannot add a chain
        CannotAddChain,

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

        /// The call to unregister_attestor failed because the address is invulnerable
        AddressIsInvulnerable,

        /// The call the urnegister_invulnerable failed because the address is not invulnerable
        AddressIsNotInvulnerable,

        /// The call to bootstrap_chain failed, the chain has previously been bootstrapped
        ChainAlreadyBootstrapped,

        /// The chain has not been bootstrapped and cannot be attested to
        ChainIsNotBootstrapped,

        /// The call to attest_block failed, the attestor is not eligible at this time
        NotEligible,

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
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::set_chain_attestation_interval())]
        pub fn set_chain_attestation_interval(
            origin: OriginFor<T>,
            chain_id: ChainId,
            chain_attestation_interval: ChainAttestationIntervalType,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                T::SupportedChains::is_chain_supported(chain_id),
                Error::<T>::ChainNotSupported
            );

            ChainAttestationInterval::<T>::set(chain_id, chain_attestation_interval);
            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::set_comittee_set_size())]
        pub fn set_comittee_set_size(
            origin: OriginFor<T>,
            new_comittee_set_size: u32,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ComitteeSetSize::<T>::put(new_comittee_set_size);

            Self::deposit_event(Event::<T>::ComitteeSetSizeChanged(new_comittee_set_size));

            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::register_attestor())]
        pub fn register_attestor(
            origin: OriginFor<T>,
            bls_public_key: BlsPublicKey,
            proof_of_possession: BlsSignature,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(
                Self::address_is_not_attestor(&who),
                Error::<T>::AlreadyAttestor
            );

            let public_key = PublicKey::from_bytes(&bls_public_key[..])
                .map_err(|_| Error::<T>::InvalidBlsPublicKey)?;

            let signature = Signature::from_bytes(&proof_of_possession[..])
                .map_err(|_| Error::<T>::InvalidBlsSignature)?;

            ensure!(
                bls_signatures::verify(
                    &signature,
                    &[bls_signatures::hash(bls_public_key[..].into())],
                    &[public_key]
                ),
                Error::<T>::InvalidProofOfPossession
            );

            Self::try_insert_attestor_and_emit_event(&who, bls_public_key)
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_attestor())]
        pub fn unregister_attestor(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;

            ensure!(Self::is_attestor(&who), Error::<T>::AddressNotAttestor);

            Self::remove_attestor_and_emit_event(&who);

            Ok(())
        }

        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_attestors())]
        pub fn set_max_attestors(origin: OriginFor<T>, new_max: u32) -> DispatchResult {
            ensure_root(origin)?;

            MaxAttestors::<T>::put(new_max);
            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::register_invulnerable())]
        pub fn register_invulnerable(
            origin: OriginFor<T>,
            attestor: T::AccountId,
            bls_public_key: BlsPublicKey,
        ) -> DispatchResult {
            ensure_root(origin)?;

            Self::try_insert_invulnerable_and_emit_event(&attestor, bls_public_key)
        }

        #[pallet::call_index(6)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_invulnerable())]
        pub fn unregister_invulnerable(
            origin: OriginFor<T>,
            attestor: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                Self::address_is_invulnerable(&attestor),
                Error::<T>::AddressIsNotInvulnerable
            );

            Self::remove_invulnerable_and_emit_event(&attestor)
        }

        #[pallet::call_index(7)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_invulnerables())]
        pub fn set_max_invulnerables(origin: OriginFor<T>, new_max: u32) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                new_max >= Invlunerables::<T>::count(),
                Error::<T>::MaxInvulnerablesCannotBeChanged
            );

            MaxInvulnerables::<T>::put(new_max);
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

            let digest = attestation.digest();

            LastDigest::<T>::set(chain_id, Some(digest));

            Attestations::<T>::insert(chain_id, digest, &attestation);

            Self::deposit_event(Event::<T>::BlockAttested(chain_id, attestation));

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

            Ok(())
        }

        #[pallet::call_index(9)]
        #[pallet::weight(<T as Config>::WeightInfo::commit_attestation(attestation.attestors.len() as u32))]
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

                if attestation.attestation.header_number < prev_block_number + interval {
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
            Attestations::<T>::insert(chain_id, digest, &attestation);

            // Update last digest
            LastDigest::<T>::set(chain_id, Some(digest));

            Self::deposit_event(Event::<T>::BlockAttested(chain_id, attestation));

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

            ensure!(
                T::SupportedChains::is_chain_supported(chain_id),
                Error::<T>::ChainNotSupported
            );

            AttestationCheckpointInterval::<T>::set(chain_id, attestations_per_checkpoint);
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
                log::info!("No attestation data provided");
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
                    let attestor_public_keys =
                        Pallet::<T>::gather_attestor_public_keys(&attestation.attestors)?;
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
        pub fn working_set_size() -> u32 {
            Attestors::<T>::count()
        }

        pub fn is_attestor(address: &T::AccountId) -> bool {
            Attestors::<T>::contains_key(address)
        }

        fn address_is_not_attestor(address: &T::AccountId) -> bool {
            !Self::is_attestor(address)
        }

        pub fn last_digest(chain_id: ChainId) -> Option<Digest> {
            LastDigest::<T>::get(chain_id)
        }

        pub fn contains_digest(chain_id: ChainId, digest: Digest) -> bool {
            Attestations::<T>::contains_key(chain_id, digest)
        }

        pub fn attestor_bls_pubkey(address: &T::AccountId) -> Option<BlsPublicKey> {
            Attestors::<T>::get(address)
        }

        fn attestor_list_has_space() -> bool {
            Attestors::<T>::count() < MaxAttestors::<T>::get()
        }

        pub fn get(
            chain_id: ChainId,
            digest: Digest,
        ) -> Option<SignedAttestation<T::Hash, T::AccountId>> {
            Attestations::<T>::get(chain_id, digest)
        }

        fn try_insert_attestor_and_emit_event(
            address: &T::AccountId,
            bls_public_key: BlsPublicKey,
        ) -> DispatchResult {
            ensure!(
                Self::attestor_list_has_space(),
                Error::<T>::AttestorListFull
            );
            Attestors::<T>::insert(address, bls_public_key);
            Self::deposit_event(Event::<T>::AttestorRegistered(address.clone()));
            Ok(())
        }

        fn vulnerable_list_has_space() -> bool {
            Invlunerables::<T>::count() < MaxInvulnerables::<T>::get()
        }

        /// Insert address as attestor & invulnerable
        fn try_insert_invulnerable_and_emit_event(
            address: &T::AccountId,
            bls_public_key: BlsPublicKey,
        ) -> DispatchResult {
            Self::try_insert_attestor_and_emit_event(address, bls_public_key)?;

            ensure!(
                Self::vulnerable_list_has_space(),
                Error::<T>::InvulnerableListFull
            );

            Invlunerables::<T>::insert(address, true);
            Self::deposit_event(Event::<T>::InvulnerableRegistered(address.clone()));
            Ok(())
        }

        fn address_is_invulnerable(address: &T::AccountId) -> bool {
            Invlunerables::<T>::contains_key(address)
        }

        fn remove_attestor_and_emit_event(address: &T::AccountId) {
            Attestors::<T>::remove(address.clone());
            Self::deposit_event(Event::<T>::AttestorUnregistered(address.clone()));
        }

        // Remove address as invulnerable and attestor
        fn remove_invulnerable_and_emit_event(address: &T::AccountId) -> DispatchResult {
            Self::remove_attestor_and_emit_event(address);

            // Remove from invulnerables
            Invlunerables::<T>::remove(address);
            Self::deposit_event(Event::<T>::InvulnerableUnregistered(address.clone()));

            Ok(())
        }

        fn validate_attestation(
            attestation: &SignedAttestation<T::Hash, T::AccountId>,
        ) -> Result<(), InherentError> {
            Self::check_duplicate(attestation)?;
            let agg_signature = Self::extract_agg_signature(&attestation.signature)?;
            let attestor_public_keys = Self::gather_attestor_public_keys(&attestation.attestors)?;
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
            let num_to_condense = T::DefaultAttestationsPerCheckpoint::get();
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
            attestors: &[T::AccountId],
        ) -> Result<Vec<PublicKey>, InherentError> {
            attestors
                .iter()
                .map(|attestor| {
                    Attestors::<T>::get(attestor)
                        .ok_or_else(|| {
                            log::error!("Attestor {:?} not found", attestor);
                            InherentError::NotValid
                        })
                        .and_then(|key_bytes| {
                            PublicKey::from_bytes(&key_bytes[..]).map_err(|_| {
                                log::error!("Invalid BLS key for attestor {:?}", attestor);
                                InherentError::NotValid
                            })
                        })
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
