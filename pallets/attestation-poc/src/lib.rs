#![cfg_attr(not(feature = "std"), no_std)]

mod types;
pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    use crate::types::{Attestation, BlockNumber, Digest};
    use attestor_primitives::{
        BlsPublicKey, ChainId, InherentError, SignedAttestation, INHERENT_IDENTIFIER,
    };
    use frame_support::pallet_prelude::{
        CountedStorageMap, DispatchResult, OptionQuery, ValueQuery,
    };
    use frame_support::{pallet_prelude::*, Blake2_128Concat};
    use frame_system::pallet_prelude::*;
    use log::debug;
    use parity_scale_codec::FullCodec;
    use sp_std::{fmt::Debug, vec::Vec};

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        // TODO: this is currently unused
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
    }

    pub trait WeightInfo {
        fn register_attestor() -> Weight;
        fn unregister_attestor() -> Weight;
        fn set_max_attestors() -> Weight;
        fn register_invulnerable() -> Weight;
        fn unregister_invulnerable() -> Weight;
        fn set_max_invulnerables() -> Weight;
        fn attest_block() -> Weight;
        fn bootstrap_chain() -> Weight;
        fn commit_attestation() -> Weight;
        fn set_comitte_set_size() -> Weight;
        fn add_supported_chain() -> Weight;
        fn remove_supported_chain() -> Weight;
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
        // TODO: figure out how to do this from the value set in the runtime config
        // T::MaxAttestationNodes
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
        BlockNumber,
        Attestation<T::Hash>,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn last_attesation_digest)]
    pub type LastDigest<T: Config> = StorageMap<_, Blake2_128Concat, ChainId, Digest, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn comittee_set_size)]
    pub type ComitteeSetSize<T: Config> = StorageValue<_, u32, ValueQuery>;

    pub type SupportedChainsVec = BoundedVec<ChainId, ConstU32<256>>;

    #[pallet::storage]
    #[pallet::getter(fn supported_chains)]
    pub type SupportedChains<T: Config> = StorageValue<_, SupportedChainsVec, ValueQuery>;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub comittee_set_size: u32,
        pub invulnerables: Vec<T::AccountId>,
        pub supported_chains: Vec<ChainId>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let comittee_set_size = &self.comittee_set_size;
            ComitteeSetSize::<T>::put(comittee_set_size);

            let invulnerables = &self.invulnerables;
            for invulnerable in invulnerables.iter() {
                Invlunerables::<T>::insert(invulnerable, true);
                Attestors::<T>::insert(
                    invulnerable,
                    [
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    ],
                );
            }

            let mut chains: SupportedChainsVec = BoundedVec::new();
            for chain in self.supported_chains.clone().into_iter() {
                chains.try_push(chain).unwrap();
            }
            SupportedChains::<T>::set(chains);
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

        ChainBootstrapped(ChainId, Attestation<T::Hash>),

        BlockAttested(ChainId, Attestation<T::Hash>),

        CheckpointReached(ChainId, Attestation<T::Hash>),

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

        /// The call to set_max_attestor_failed, most likely because the current list is longer than the new requested maximum
        MaxAttestorsCannotBeChanged,

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
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::add_supported_chain())]
        pub fn add_supported_chain(origin: OriginFor<T>, chain_id: ChainId) -> DispatchResult {
            ensure_root(origin)?;

            let mut chains = SupportedChains::<T>::get();
            chains
                .try_push(chain_id)
                .map_err(|_| Error::<T>::CannotAddChain)?;

            SupportedChains::<T>::set(chains);

            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::set_comitte_set_size())]
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
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(
                Self::address_is_not_attestor(&who),
                Error::<T>::AlreadyAttestor
            );

            Self::try_insert_attestor_and_emit_event(&who, bls_public_key)
        }

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_attestor())]
        pub fn unregister_attestor(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(Self::is_attestor(&who), Error::<T>::AddressNotAttestor);

            Self::remove_attestor_and_emit_event(&who);

            Ok(())
        }

        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_attestors())]
        pub fn set_max_attestors(origin: OriginFor<T>, new_max: u32) -> DispatchResult {
            ensure_root(origin)?;

            let count = Attestors::<T>::count();

            if count == 0 {
                MaxAttestors::<T>::put(new_max);
                return Ok(());
            }

            ensure!(new_max >= count, Error::<T>::MaxAttestorsCannotBeChanged);

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
                new_max < Invlunerables::<T>::count(),
                Error::<T>::MaxInvulnerablesCannotBeChanged
            );

            MaxInvulnerables::<T>::put(new_max);
            Ok(())
        }

        #[pallet::call_index(8)]
        #[pallet::weight(<T as Config>::WeightInfo::bootstrap_chain())]
        pub fn bootstrap_chain(
            origin: OriginFor<T>,
            chain_id: ChainId,
            block_number: BlockNumber,
            attestation: SignedAttestation<T::Hash>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            let digest = attestation.digest;
            LastDigest::<T>::set(chain_id, Some(digest));

            let attestation_insert: Attestation<T::Hash> =
                Attestation::new(attestation.clone(), digest);
            Attestations::<T>::insert(chain_id, block_number, &attestation_insert);

            Self::deposit_event(Event::<T>::ChainBootstrapped(chain_id, attestation_insert));
            Ok(())
        }

        #[pallet::call_index(9)]
        #[pallet::weight(<T as Config>::WeightInfo::commit_attestation())]
        pub fn commit_attestation(
            origin: OriginFor<T>,
            attestation: SignedAttestation<T::Hash>,
        ) -> DispatchResult {
            ensure_none(origin)?;

            let digest = if let Some(digest) = LastDigest::<T>::get(attestation.chain_id()) {
                digest
            } else {
                debug!("First attestation for chain, otherwise something is wrong");
                attestation.digest
            };

            let block_number = attestation.attestation_data.header_number;
            let attestation_insert = Attestation::new(attestation.clone(), digest);
            Attestations::<T>::insert(attestation.chain_id(), block_number, &attestation_insert);

            // Update last digest
            LastDigest::<T>::set(attestation.chain_id(), Some(attestation.digest));

            Self::deposit_event(Event::<T>::BlockAttested(
                attestation.chain_id(),
                attestation_insert,
            ));

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
                .get_data::<SignedAttestation<T::Hash>>(&INHERENT_IDENTIFIER)
                .expect("Attestation inherent data not correctly encoded");

            // Check if atleast the attestation was not already submitted
            if let Some(attestation) = inherent_data {
                if let Some(digest) = LastDigest::<T>::get(attestation.attestation_data.chain_id) {
                    if digest == attestation.digest {
                        log::error!("Attestation with digest: {:?} is duplicate", digest);
                        return None;
                    }
                };

                Some(Call::commit_attestation { attestation })
            } else {
                log::info!("No attestation data provided");
                None
            }
        }

        fn check_inherent(
            _call: &Self::Call,
            data: &InherentData,
        ) -> sp_std::result::Result<(), Self::Error> {
            let inherent_data = data
                .get_data::<SignedAttestation<T::Hash>>(&INHERENT_IDENTIFIER)
                .expect("Timestamp inherent data not correctly encoded");

            // Check if atleast the attestation was not already submitted
            if let Some(attestation) = inherent_data {
                if let Some(digest) = LastDigest::<T>::get(attestation.attestation_data.chain_id) {
                    if digest == attestation.attestation_data.digest() {
                        log::error!("Attestation with digest: {:?} is duplicate", digest);
                        return Err(InherentError::Duplicate);
                    }
                }
            }

            Ok(())
        }

        fn is_inherent(call: &Self::Call) -> bool {
            matches!(call, Call::commit_attestation { .. })
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn is_attestor(address: &T::AccountId) -> bool {
            Attestors::<T>::contains_key(address)
        }

        pub fn address_is_not_attestor(address: &T::AccountId) -> bool {
            !Self::is_attestor(address)
        }

        pub fn last_digest(chain_id: u8) -> Option<Digest> {
            LastDigest::<T>::get(chain_id)
        }

        pub fn attestor_list_has_space() -> bool {
            Attestors::<T>::count() < MaxAttestors::<T>::get()
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
    }
}
