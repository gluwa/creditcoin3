#![cfg_attr(not(feature = "std"), no_std)]

mod types;
pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

mod benchmarking;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
    use crate::types::{BlockNumber, BlockSerializable};
    use frame_support::inherent::{InherentIdentifier, IsFatalError};
    use frame_support::pallet_prelude::{
        CountedStorageMap, DispatchResult, OptionQuery, ValueQuery,
    };
    use frame_support::{pallet_prelude::*, Blake2_128Concat};
    use frame_system::pallet_prelude::*;
    use sp_std::vec::Vec;

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
        fn set() -> Weight;
        fn set_comitte_set_size() -> Weight;
    }

    #[pallet::storage]
    #[pallet::getter(fn attestors)]
    pub type Attestors<T: Config> = CountedStorageMap<
        Hasher = Blake2_128Concat,
        Key = T::AccountId,
        // Value can be ignored
        Value = bool,
        QueryKind = ValueQuery,
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
    #[pallet::getter(fn last_block)]
    pub type LastBlock<T: Config> = StorageValue<_, BlockSerializable, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn commitments)]
    pub type Commitments<T: Config> = CountedStorageMap<
        Hasher = Blake2_128Concat,
        Key = BlockNumber,
        // Value can be ignored
        Value = BlockSerializable,
        QueryKind = OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn comittee_set_size)]
    pub type ComitteeSetSize<T: Config> = StorageValue<_, u32, ValueQuery>;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub comittee_set_size: u32,
        pub invulnerables: Vec<T::AccountId>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            let comittee_set_size = &self.comittee_set_size;
            ComitteeSetSize::<T>::put(comittee_set_size);

            let invulnerables = &self.invulnerables;
            for invulnerable in invulnerables.iter() {
                Invlunerables::<T>::insert(invulnerable, true);
                Attestors::<T>::insert(invulnerable, true);
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

        ChainBootstrapped(BlockSerializable),

        BlockAttested(BlockSerializable),

        CheckpointReached(BlockSerializable),

        ComitteeSetSizeChanged(u32),
    }

    #[pallet::error]
    pub enum Error<T> {
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
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::register_attestor())]
        pub fn register_attestor(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(
                Self::address_is_not_attestor(&who),
                Error::<T>::AlreadyAttestor
            );

            Self::try_insert_attestor_and_emit_event(&who)
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_attestor())]
        pub fn unregister_attestor(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(
                Self::address_is_attestor(&who),
                Error::<T>::AddressNotAttestor
            );

            Self::remove_attestor_and_emit_event(&who);

            Ok(())
        }

        #[pallet::call_index(2)]
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

        #[pallet::call_index(3)]
        #[pallet::weight(<T as Config>::WeightInfo::register_invulnerable())]
        pub fn register_invulnerable(
            origin: OriginFor<T>,
            attestor: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;

            Self::try_insert_invulnerable_and_emit_event(&attestor)
        }

        #[pallet::call_index(4)]
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

        #[pallet::call_index(5)]
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

        #[pallet::call_index(6)]
        #[pallet::weight(<T as Config>::WeightInfo::attest_block())]
        pub fn attest_block(
            origin: OriginFor<T>,
            block: BlockSerializable,
            eligibility: u64,
        ) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            ensure!(
                Self::chain_is_bootstrapped(),
                Error::<T>::ChainIsNotBootstrapped,
            );

            ensure!(
                Self::is_eligible_for_attestation(&block, eligibility),
                Error::<T>::NotEligible,
            );

            ensure!(Self::is_block_valid(&block), Error::<T>::InvalidAttestation,);

            LastBlock::<T>::set(Some(block.clone()));
            Self::deposit_event(Event::<T>::BlockAttested(block.clone()));

            if block.block_number % T::CommittmentInterval::get() == 0 {
                Commitments::<T>::insert(block.block_number, block.clone());
                Self::deposit_event(Event::<T>::CheckpointReached(block))
            }

            Ok(())
        }

        #[pallet::call_index(7)]
        #[pallet::weight(<T as Config>::WeightInfo::bootstrap_chain())]
        pub fn bootstrap_chain(origin: OriginFor<T>, block: BlockSerializable) -> DispatchResult {
            ensure_root(origin)?;

            ensure!(
                !Self::chain_is_bootstrapped(),
                Error::<T>::ChainAlreadyBootstrapped
            );

            LastBlock::<T>::put(block.clone());
            Commitments::<T>::insert(block.block_number, block.clone());

            Self::deposit_event(Event::<T>::ChainBootstrapped(block));
            Ok(())
        }

        #[pallet::call_index(8)]
        #[pallet::weight(<T as Config>::WeightInfo::set())]
        pub fn set(origin: OriginFor<T>, _attestation: InherentType) -> DispatchResult {
            ensure_none(origin)?;

            Ok(())
        }

        #[pallet::call_index(9)]
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
    }

    pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"attest0r";

    // TODO: should be some kind of storage structure that has:
    // - BLS aggreated signatures
    // - Attestors that were included in the creation of the signature
    // Eventually only store the signature
    pub type InherentType = sp_std::vec::Vec<u8>;

    #[derive(Encode, sp_runtime::RuntimeDebug)]
    #[cfg_attr(feature = "std", derive(Decode))]
    pub enum InherentError {
        NotValid,
        Duplicate,
    }

    impl IsFatalError for InherentError {
        fn is_fatal_error(&self) -> bool {
            match self {
                InherentError::NotValid => true,
                InherentError::Duplicate => true,
            }
        }
    }

    #[pallet::inherent]
    impl<T: Config> ProvideInherent for Pallet<T> {
        type Call = Call<T>;
        type Error = InherentError;
        const INHERENT_IDENTIFIER: InherentIdentifier = INHERENT_IDENTIFIER;

        fn create_inherent(data: &InherentData) -> Option<Self::Call> {
            let attestation = data
                .get_data::<InherentType>(&INHERENT_IDENTIFIER)
                .expect("Attestation inherent data not correctly encoded");
            // .expect("Attestation inherent data must be provided");

            attestation.map(|attestation| Call::set { attestation })

            // let next_time = cmp::max(data, Self::now() + T::MinimumPeriod::get());
            // Some(Call::set { now: next_time })
        }

        fn check_inherent(
            _call: &Self::Call,
            data: &InherentData,
        ) -> sp_std::result::Result<(), Self::Error> {
            let _data = data
                .get_data::<InherentType>(&INHERENT_IDENTIFIER)
                .expect("Timestamp inherent data not correctly encoded");
            // .expect("Timestamp inherent data must be provided");

            // TODO: verify again some basic things

            Ok(())
        }

        fn is_inherent(call: &Self::Call) -> bool {
            matches!(call, Call::set { .. })
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn address_is_attestor(address: &T::AccountId) -> bool {
            Attestors::<T>::contains_key(address)
        }

        pub fn address_is_not_attestor(address: &T::AccountId) -> bool {
            !Self::address_is_attestor(address)
        }

        pub fn attestor_list_has_space() -> bool {
            Attestors::<T>::count() < MaxAttestors::<T>::get()
        }

        fn try_insert_attestor_and_emit_event(address: &T::AccountId) -> DispatchResult {
            ensure!(
                Self::attestor_list_has_space(),
                Error::<T>::AttestorListFull
            );
            Attestors::<T>::insert(address, true);
            Self::deposit_event(Event::<T>::AttestorRegistered(address.clone()));
            Ok(())
        }

        fn vulnerable_list_has_space() -> bool {
            Invlunerables::<T>::count() < MaxInvulnerables::<T>::get()
        }

        /// Insert address as attestor & invulnerable
        fn try_insert_invulnerable_and_emit_event(address: &T::AccountId) -> DispatchResult {
            Self::try_insert_attestor_and_emit_event(address)?;

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

        fn chain_is_bootstrapped() -> bool {
            let first_block = Self::last_block();

            let mut first_commitment = Commitments::<T>::iter();
            let first_commitment = first_commitment.next();

            first_block.is_some() && first_commitment.is_some()
        }

        fn is_eligible_for_attestation(_block: &BlockSerializable, _eligiblity: u64) -> bool {
            true
        }

        fn is_block_valid(block: &BlockSerializable) -> bool {
            // unwrap here because we check if the chain is bootstrapped in the extrinsic
            let last_block = Self::last_block().unwrap();

            last_block.digest == block.prev_digest
        }
    }
}
