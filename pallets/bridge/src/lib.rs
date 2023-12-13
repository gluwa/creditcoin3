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
    use crate::types::{BalanceFor, Cc2BurnId, CollectionInfo};
    use frame_support::dispatch::PostDispatchInfo;
    use frame_support::pallet_prelude::DispatchResult;
    use frame_support::traits::Currency;
    use frame_support::{pallet_prelude::*, Twox64Concat};
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{BlockNumberProvider, Zero};

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        type Currency: Currency<Self::AccountId>;

        type WeightInfo: WeightInfo;
    }

    pub trait WeightInfo {
        fn add_authority() -> Weight;
        fn approve_collection() -> Weight;
        fn remove_authority() -> Weight;
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        FundsCollected(Cc2BurnId, T::AccountId, BalanceFor<T>),
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Collection already completed
        AlreadyCollected,
        /// Invalid collection amount
        InvalidCollectionAmount,
        /// Not an authority
        NotAnAuthority,
        /// Already an authority
        AlreadyAuthority,
        /// Insufficient authority
        InsufficientAuthority,
    }

    #[pallet::storage]
    #[pallet::getter(fn collections)]
    pub(super) type Collections<T: Config> = StorageMap<
        _,
        Twox64Concat,
        Cc2BurnId,
        CollectionInfo<T::AccountId, BalanceFor<T>, BlockNumberFor<T>>,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn authorities)]
    pub type Authorities<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, ()>;

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(<T as Config>::WeightInfo::approve_collection())]
        pub fn approve_collection(
            origin: OriginFor<T>,
            burn_id: Cc2BurnId,
            collector: T::AccountId,
            amount: BalanceFor<T>,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(Self::is_authority(&who), Error::<T>::InsufficientAuthority);

            Self::approve_collection_cc2(origin.clone(), burn_id, collector, amount)
        }

        #[pallet::call_index(1)]
        #[pallet::weight(<T as Config>::WeightInfo::add_authority())]
        pub fn add_authority(
            origin: OriginFor<T>,
            who: T::AccountId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;

            ensure!(!Self::is_authority(&who), Error::<T>::AlreadyAuthority);

            Self::insert_authority(&who);

            Ok(PostDispatchInfo {
                actual_weight: None,
                pays_fee: Pays::No,
            })
        }

        #[pallet::call_index(2)]
        #[pallet::weight(<T as Config>::WeightInfo::remove_authority())]
        pub fn remove_authority(
            origin: OriginFor<T>,
            who: T::AccountId,
        ) -> DispatchResultWithPostInfo {
            ensure_root(origin)?;

            ensure!(Self::is_authority(&who), Error::<T>::NotAnAuthority);

            Self::delete_authority(&who);

            Ok(PostDispatchInfo {
                actual_weight: None,
                pays_fee: Pays::No,
            })
        }
    }

    impl<T: Config> Pallet<T> {
        fn approve_collection_cc2(
            origin: OriginFor<T>,
            burn_id: Cc2BurnId,
            collector: T::AccountId,
            amount: BalanceFor<T>,
        ) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            ensure!(
                Self::collections(&burn_id).is_none(),
                Error::<T>::AlreadyCollected
            );

            ensure!(!amount.is_zero(), Error::<T>::InvalidCollectionAmount);
            Self::mint_into(&collector, amount);

            let info = CollectionInfo {
                block_number: Self::block_number(),
                amount,
                collector: collector.clone(),
            };

            Collections::<T>::insert(&burn_id, info);

            Self::deposit_event(Event::<T>::FundsCollected(burn_id, collector, amount));
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        fn block_number() -> BlockNumberFor<T> {
            frame_system::Pallet::<T>::current_block_number()
        }
        fn mint_into(who: &T::AccountId, amount: BalanceFor<T>) {
            let minted = <T::Currency as Currency<T::AccountId>>::issue(amount);
            <T::Currency as Currency<T::AccountId>>::resolve_creating(who, minted);
        }
        fn is_authority(authority: &T::AccountId) -> bool {
            Authorities::<T>::contains_key(authority)
        }
        fn insert_authority(authority: &T::AccountId) {
            Authorities::<T>::insert(authority, ());
        }
        fn delete_authority(authority: &T::AccountId) {
            Authorities::<T>::remove(authority);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Balances, Bridge, ExtBuilder, RuntimeOrigin, System, Test, COLLECTOR};
    use crate::types::{Cc2BurnId, CollectionInfo};
    use assert_matches::assert_matches;

    use frame_support::{assert_err, assert_ok};
    use sp_runtime::traits::BadOrigin;

    #[test]
    fn approve_collection_cc2_should_error_when_collection_completed() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let burn_id = Cc2BurnId(1);

            let completed = CollectionInfo {
                amount: 100,
                block_number: 1,
                collector: COLLECTOR,
            };
            Collections::<Test>::insert(&burn_id, completed);

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), COLLECTOR));

            let expected_error = Error::<Test>::AlreadyCollected;
            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(COLLECTOR), burn_id, COLLECTOR, 0),
                expected_error
            );
        })
    }

    #[test]
    fn approve_collection_cc2_should_update_balance_and_emit_event_when_successful() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let burn_id = Cc2BurnId(1);
            let prior_balance = Balances::free_balance(COLLECTOR);

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), COLLECTOR));

            let amount = 100;
            assert_ok!(Bridge::approve_collection(
                RuntimeOrigin::signed(COLLECTOR),
                burn_id.clone(),
                COLLECTOR,
                amount
            ));

            let ending_balance = Balances::free_balance(COLLECTOR);
            assert!(ending_balance > prior_balance);

            let event = <frame_system::Pallet<Test>>::events().pop().expect("an event").event;
            assert_matches!(
                    event,
                    crate::mock::RuntimeEvent::Bridge(crate::Event::<Test>::FundsCollected(actual_burn_id, actual_collector, actual_amount)) => {
                            assert_eq!(actual_burn_id, burn_id);
                            assert_eq!(actual_collector, COLLECTOR);
                            assert_eq!(actual_amount, amount);
                    }
            );
        })
    }

    #[test]
    fn approve_collection_cc2_should_error_when_amount_is_invalid() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let burn_id = Cc2BurnId(1);

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), COLLECTOR));

            let expected_error = Error::<Test>::InvalidCollectionAmount;
            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(COLLECTOR), burn_id, COLLECTOR, 0),
                expected_error,
            );
        })
    }

    #[test]
    fn add_authority_should_work() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_none());

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), COLLECTOR));

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_some());
        })
    }

    #[test]
    fn add_authority_should_error_when_not_signed_by_root() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_none());

            assert_err!(
                Bridge::add_authority(RuntimeOrigin::signed(1), COLLECTOR),
                BadOrigin
            );
        })
    }

    #[test]
    fn remove_authority_should_error_when_not_signed_by_root() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_none());

            assert_err!(
                Bridge::remove_authority(RuntimeOrigin::signed(1), COLLECTOR),
                BadOrigin
            );
        })
    }

    #[test]
    fn add_authority_should_error_when_called_with_existing_authority() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_none());

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), COLLECTOR));
            assert_err!(
                Bridge::add_authority(RuntimeOrigin::root(), COLLECTOR),
                Error::<Test>::AlreadyAuthority
            );
        })
    }

    #[test]
    fn remove_authority_should_error_when_called_with_nonexisting_authority() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_none());

            assert_err!(
                Bridge::remove_authority(RuntimeOrigin::root(), COLLECTOR),
                Error::<Test>::NotAnAuthority
            );
        })
    }

    #[test]
    fn approve_collection_cc2_should_error_with_insufficient_authority() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let burn_id = Cc2BurnId(1);

            let authority = Bridge::authorities(COLLECTOR);
            assert!(authority.is_none());

            let expected_err = Error::<Test>::InsufficientAuthority;

            assert_err!(
                Bridge::approve_collection(
                    RuntimeOrigin::signed(COLLECTOR),
                    burn_id,
                    COLLECTOR,
                    100
                ),
                expected_err
            );
        })
    }
}
