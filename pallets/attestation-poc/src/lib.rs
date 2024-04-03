#![cfg_attr(not(feature = "std"), no_std)]

mod types;
pub use pallet::*;

#[allow(clippy::unnecessary_cast)]
pub mod weights;

mod benchmarking;

#[cfg(test)]
mod mock;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::dispatch::PostDispatchInfo;
    use frame_support::pallet_prelude::{CountedStorageMap, DispatchResult, OptionQuery};
    use frame_support::traits::Currency;
    use frame_support::{pallet_prelude::*, Blake2_128Concat, Twox64Concat};
    use frame_system::pallet_prelude::*;
    use log::info;
    use sp_runtime::traits::{BlockNumberProvider, Zero};

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type WeightInfo: WeightInfo;
        // TODO: this is currently unused
        type MaxAttestestationNodes: Get<u32>;
    }

    pub trait WeightInfo {
        fn register_attestor() -> Weight;
        fn unregister_attestor() -> Weight;
        fn set_max_attestors() -> Weight;
        fn register_invulnerable() -> Weight;
        fn unregister_invulernable() -> Weight;
        fn set_max_invulnerables() -> Weight;
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
        100
    }

    #[pallet::type_value]
    pub fn MaxInvulernablesDefault<T: Config>() -> u32 {
        100
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// Emitted when an attestor is properly registered with the attestation system
        AttestorRegistered(T::AccountId),

        AttestorUnregistered(T::AccountId),

        /// Emitted when an invulnerable is properly registered with the attestation system
        InvulnerableRegistered(T::AccountId),

        InvulnerableUnregistered(T::AccountId),
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

            ensure!(
                Self::address_is_not_invulnerable(&who),
                Error::<T>::AddressIsInvulnerable
            );

            Self::remove_attestor_and_emit_event(&who);

            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_attestors())]
        pub fn set_max_attestors(origin: OriginFor<T>, new_max: u32) -> DispatchResult {
            let _ = ensure_root(origin)?;

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
        pub fn register_invulnerable(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            // notice the not compared to the similar check in register_attestor
            ensure!(
                Self::address_is_attestor(&who),
                Error::<T>::AddressNotAttestor,
            );

            Self::try_insert_invulnerable_ane_emit_event(&who)
        }

        #[pallet::call_index(4)]
        #[pallet::weight(<T as Config>::WeightInfo::unregister_invulernable())]
        pub fn unregister_invulernable(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(
                Self::address_is_attestor(&who),
                Error::<T>::AddressNotAttestor
            );

            ensure!(
                Self::address_is_invulnerable(&who),
                Error::<T>::AddressIsNotInvulnerable
            );

            Self::remove_invulnerable_and_emit_event(&who);

            Ok(())
        }

        #[pallet::call_index(5)]
        #[pallet::weight(<T as Config>::WeightInfo::set_max_invulnerables())]
        pub fn set_max_invulnerables(origin: OriginFor<T>, new_max: u32) -> DispatchResult {
            let _ = ensure_root(origin)?;

            ensure!(
                new_max < Invlunerables::<T>::count(),
                Error::<T>::MaxInvulnerablesCannotBeChanged
            );

            MaxInvulnerables::<T>::put(new_max);
            Ok(())
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

        fn try_insert_invulnerable_ane_emit_event(address: &T::AccountId) -> DispatchResult {
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

        fn address_is_not_invulnerable(address: &T::AccountId) -> bool {
            !Self::address_is_invulnerable(address)
        }

        fn remove_attestor_and_emit_event(address: &T::AccountId) {
            Attestors::<T>::remove(address.clone());
            Self::deposit_event(Event::<T>::AttestorUnregistered(address.clone()));
        }

        fn remove_invulnerable_and_emit_event(address: &T::AccountId) {
            Invlunerables::<T>::remove(address);
            Self::deposit_event(Event::<T>::InvulnerableUnregistered(address.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Attestation, ExtBuilder, RuntimeOrigin, Test, ATTESTOR_1, ATTESTOR_2};
    use assert_matches::assert_matches;
    use frame_support::{assert_err, assert_ok};
    use sp_runtime::traits::BadOrigin;

    fn attestor_1() -> RuntimeOrigin {
        RuntimeOrigin::signed(ATTESTOR_1)
    }

    fn attestor_2() -> RuntimeOrigin {
        RuntimeOrigin::signed(ATTESTOR_2)
    }

    #[test]
    fn register_attestor_should_work_happy_path() {
        ExtBuilder.build_and_execute(|| {
            assert_ok!(Attestation::register_attestor(RuntimeOrigin::signed(
                ATTESTOR_1
            )));
        })
    }

    #[test]
    fn register_attestor_should_fail_when_address_is_already_registered() {
        ExtBuilder.build_and_execute(|| {
            assert_ok!(Attestation::register_attestor(RuntimeOrigin::signed(
                ATTESTOR_1
            )));

            assert_err!(
                Attestation::register_attestor(RuntimeOrigin::signed(ATTESTOR_1)),
                Error::<Test>::AlreadyAttestor
            );
        })
    }

    #[test]
    fn register_attestor_should_fail_when_list_is_full() {
        ExtBuilder.build_and_execute(|| {
            let root = RuntimeOrigin::root();
            let attestor_1 = attestor_1();
            let attestor_2 = attestor_2();

            assert_ok!(Attestation::set_max_attestors(root, 1));
            assert_ok!(Attestation::register_attestor(attestor_1));
            assert_err!(
                Attestation::register_attestor(attestor_2),
                Error::<Test>::AttestorListFull
            );
        })
    }

    // TODO: make this smarter and rely on the runtime value instead of the function
    #[test]
    fn max_attestor_default_should_be_100() {
        ExtBuilder.build_and_execute(|| assert_matches!(Attestation::max_attestors(), 100))
    }

    #[test]
    fn max_invulnerable_default_should_be_100() {
        ExtBuilder.build_and_execute(|| assert_matches!(Attestation::max_invulnerables(), 100))
    }

    #[test]
    fn set_max_attestors_should_error_with_non_root_origin() {
        ExtBuilder.build_and_execute(|| {
            let bad_origin = attestor_1();
            assert_err!(Attestation::set_max_attestors(bad_origin, 1), BadOrigin)
        })
    }

    #[test]
    fn set_max_invulnerables_should_error_with_non_root_origin() {
        ExtBuilder.build_and_execute(|| {
            let bad_origin = attestor_1();
            assert_err!(
                Attestation::set_max_invulnerables(bad_origin, 200),
                BadOrigin
            )
        })
    }

    #[test]
    fn set_max_attestors_should_error_if_list_is_truncated() {
        ExtBuilder.build_and_execute(|| {
            let attestor_1 = attestor_1();
            let attestor_2 = attestor_2();
            assert_ok!(Attestation::register_attestor(attestor_1));
            assert_ok!(Attestation::register_attestor(attestor_2));
            assert_err!(
                Attestation::set_max_attestors(RuntimeOrigin::root(), 1),
                Error::<Test>::MaxAttestorsCannotBeChanged
            );
        })
    }

    #[test]
    fn unregister_attestor_should_work_happy_path() {
        ExtBuilder.build_and_execute(|| {
            let attestor = attestor_1();
            assert_ok!(Attestation::register_attestor(attestor.clone()));
            assert_ok!(Attestation::unregister_attestor(attestor));
        })
    }

    #[test]
    fn unregister_attestor_should_fail_when_address_is_not_registered() {
        ExtBuilder.build_and_execute(|| {
            let attestor = attestor_1();
            assert_err!(
                Attestation::unregister_attestor(attestor),
                Error::<Test>::AddressNotAttestor
            );
        })
    }
    #[test]
    fn unregister_attestor_should_fail_when_address_is_invulnerable() {
        ExtBuilder.build_and_execute(|| {
            let attestor = attestor_1();
            assert_ok!(Attestation::register_attestor(attestor.clone()));
            assert_ok!(Attestation::register_invulnerable(attestor.clone()));
            assert_err!(
                Attestation::unregister_attestor(attestor),
                Error::<Test>::AddressIsInvulnerable
            );
        })
    }

    #[test]
    fn unregister_invulnerable_should_work_happy_path() {
        ExtBuilder.build_and_execute(|| {
            let attestor = attestor_1();
            assert_ok!(Attestation::register_attestor(attestor.clone()));

            assert_ok!(Attestation::register_invulnerable(attestor.clone()));
            assert_ok!(Attestation::unregister_invulernable(attestor));
        })
    }

    #[test]
    fn unregister_invulnerable_should_fail_when_address_is_not_registered() {
        ExtBuilder.build_and_execute(|| {
            let attestor = attestor_1();
            assert_err!(
                Attestation::unregister_invulernable(attestor),
                Error::<Test>::AddressNotAttestor
            );
        })
    }
    #[test]
    fn unregister_invulnerable_should_fail_when_address_is_not_invulnerable() {
        ExtBuilder.build_and_execute(|| {
            let attestor = attestor_1();
            assert_ok!(Attestation::register_attestor(attestor.clone()));
            assert_err!(
                Attestation::unregister_invulernable(attestor),
                Error::<Test>::AddressIsNotInvulnerable
            );
        })
    }
}
