#![cfg_attr(not(feature = "std"), no_std)]

mod types;
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use crate::types::{BurnId, CollectionInfo, CollectionStatus, FailureReason};
    use frame_support::dispatch::PostDispatchInfo;
    use frame_support::pallet_prelude::DispatchResult;
    use frame_support::traits::{fungible::Mutate, ReservableCurrency};
    use frame_support::{pallet_prelude::*, Twox64Concat};
    use frame_system::pallet_prelude::*;
    use sp_runtime::SaturatedConversion;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_balances::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        type Currency: ReservableCurrency<Self::AccountId>;
    }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        CollectionInitiated(BurnId),
        FundsCollected(BurnId, T::AccountId, T::Balance),
        CollectionFailed(BurnId, FailureReason),
        CollectionExpired,
    }

    #[pallet::error]
    pub enum Error<T> {
        AlreadyCollected,
        AlreadyInProgress,
        CollectionNotFound,
        CollectionNotInProgress,
        InvalidCollectionAmount,
        NotAnAuthority,
        AlreadyAuthority,
        InsufficientAuthority,
    }

    #[pallet::storage]
    #[pallet::getter(fn collections)]
    pub(super) type Collections<T: Config> =
        StorageMap<_, Twox64Concat, BurnId, CollectionInfo, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn in_progress)]
    pub(super) type InProgress<T: Config> =
        StorageMap<_, Twox64Concat, BurnId, CollectionInfo, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn authorities)]
    pub type Authorities<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, ()>;

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(1)]
        #[pallet::weight({0_0})]
        pub fn collect_funds(origin: OriginFor<T>, burn_id: BurnId) -> DispatchResult {
            let _ = ensure_signed(origin.clone())?;

            match burn_id {
                BurnId::Creditcoin2(_) => Self::collect_funds_cc2(origin.clone(), burn_id),
            }
        }

        #[pallet::call_index(2)]
        #[pallet::weight({0_0})]
        pub fn approve_collection(
            origin: OriginFor<T>,
            burn_id: BurnId,
            collector: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(Self::is_authority(&who), Error::<T>::InsufficientAuthority);

            match burn_id {
                BurnId::Creditcoin2(_) => {
                    Self::approve_collection_cc2(origin.clone(), burn_id, collector, amount)
                }
            }
        }

        #[pallet::call_index(3)]
        #[pallet::weight({0_0})]
        pub fn reject_collection(
            origin: OriginFor<T>,
            burn_id: BurnId,
            reason: FailureReason,
        ) -> DispatchResult {
            let who = ensure_signed(origin.clone())?;

            ensure!(Self::is_authority(&who), Error::<T>::InsufficientAuthority);

            match burn_id {
                BurnId::Creditcoin2(_) => Self::reject_collection_cc2(origin, burn_id, reason),
            }
        }

        #[pallet::call_index(4)]
        #[pallet::weight({0_0})]
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

        #[pallet::call_index(5)]
        #[pallet::weight({0_0})]
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
        pub fn collect_funds_cc2(origin: OriginFor<T>, burn_id: BurnId) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let completed_collections = Self::collections(burn_id.clone());
            ensure!(
                completed_collections.is_none(),
                Error::<T>::AlreadyCollected
            );

            let in_progress_collections = Self::in_progress(burn_id.clone());
            ensure!(
                in_progress_collections.is_none(),
                Error::<T>::AlreadyInProgress
            );

            let new_attempt: CollectionInfo = Default::default();
            InProgress::<T>::insert(burn_id.clone(), new_attempt);
            Self::deposit_event(Event::<T>::CollectionInitiated(burn_id));

            Ok(())
        }

        pub fn approve_collection_cc2(
            origin: OriginFor<T>,
            burn_id: BurnId,
            collector: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let in_progress = InProgress::<T>::get(burn_id.clone());

            ensure!(in_progress.is_some(), Error::<T>::CollectionNotFound,);

            let in_progress = in_progress.expect("This should never fail");

            ensure!(
                in_progress.status != CollectionStatus::Completed,
                Error::<T>::AlreadyCollected
            );

            let completed_collection = Collections::<T>::get(burn_id.clone());
            ensure!(completed_collection.is_none(), Error::<T>::AlreadyCollected);

            let amount_128 = amount.saturated_into::<u128>();

            ensure!(amount_128.gt(&0u128), Error::<T>::InvalidCollectionAmount);
            <pallet_balances::Pallet<T> as Mutate<T::AccountId>>::mint_into(&collector, amount)?;

            let status = CollectionInfo {
                status: CollectionStatus::Completed,
                reason: None,
            };

            InProgress::<T>::remove(burn_id.clone());
            Collections::<T>::insert(burn_id.clone(), status);

            Self::deposit_event(Event::<T>::FundsCollected(burn_id, collector, amount));
            Ok(())
        }

        pub fn reject_collection_cc2(
            origin: OriginFor<T>,
            burn_id: BurnId,
            reason: FailureReason,
        ) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let in_progress = InProgress::<T>::get(burn_id.clone());

            ensure!(in_progress.is_some(), Error::<T>::CollectionNotFound);

            let in_progress = in_progress.expect("This should never fail");

            ensure!(
                in_progress.status != CollectionStatus::Completed,
                Error::<T>::AlreadyCollected
            );

            ensure!(
                in_progress.status == CollectionStatus::InProgress,
                Error::<T>::CollectionNotInProgress,
            );

            let status = CollectionInfo {
                status: CollectionStatus::Failed,
                reason: Some(reason.clone()),
            };

            InProgress::<T>::remove(burn_id.clone());
            Collections::<T>::insert(burn_id.clone(), status);

            Self::deposit_event(Event::<T>::CollectionFailed(burn_id, reason));
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
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
    use crate::{
        self as pallet_bridge,
        types::{BurnId, CollectionInfo},
    };

    use frame_support::{
        assert_err, assert_ok, ord_parameter_types,
        traits::{ConstU32, ConstU64},
    };
    use sp_core::H256;
    use sp_runtime::{
        traits::{BadOrigin, BlakeTwo256, IdentityLookup},
        BuildStorage,
    };

    type Block = frame_system::mocking::MockBlock<Test>;

    frame_support::construct_runtime!(
        pub enum Test
        {
            System: frame_system,
            Balances: pallet_balances,
            Bridge: pallet_bridge,
        }
    );

    impl frame_system::Config for Test {
        type BaseCallFilter = frame_support::traits::Everything;
        type BlockWeights = ();
        type BlockLength = ();
        type DbWeight = ();
        type RuntimeOrigin = RuntimeOrigin;
        type Nonce = u64;
        type Hash = H256;
        type RuntimeCall = RuntimeCall;
        type Hashing = BlakeTwo256;
        type AccountId = u64;
        type Lookup = IdentityLookup<Self::AccountId>;
        type Block = Block;
        type RuntimeEvent = RuntimeEvent;
        type BlockHashCount = ConstU64<250>;
        type Version = ();
        type PalletInfo = PalletInfo;
        type AccountData = pallet_balances::AccountData<u64>;
        type OnNewAccount = ();
        type OnKilledAccount = ();
        type SystemWeightInfo = ();
        type SS58Prefix = ();
        type OnSetCode = ();
        type MaxConsumers = ConstU32<16>;
    }

    impl pallet_balances::Config for Test {
        type MaxLocks = ();
        type MaxReserves = ();
        type ReserveIdentifier = [u8; 8];
        type Balance = u64;
        type RuntimeEvent = RuntimeEvent;
        type DustRemoval = ();
        type ExistentialDeposit = ConstU64<1>;
        type AccountStore = System;
        type WeightInfo = ();
        type FreezeIdentifier = ();
        type MaxFreezes = ();
        type RuntimeHoldReason = ();
        type MaxHolds = ();
    }

    ord_parameter_types! {
        pub const One: u64 = 1;
    }
    impl Config for Test {
        type RuntimeEvent = RuntimeEvent;
        type Currency = Balances;
    }

    fn new_test_ext() -> sp_io::TestExternalities {
        let mut t = frame_system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();
        pallet_balances::GenesisConfig::<Test> {
            balances: vec![(1, 10), (2, 10)],
        }
        .assimilate_storage(&mut t)
        .unwrap();
        t.into()
    }

    #[test]
    fn collect_funds_should_create_new_collection_when_burn_id_nonexisting() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);

            let existing_attempt = Collections::<Test>::get(burn_id.clone());
            assert!(existing_attempt.is_none());

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            System::assert_has_event(Event::<Test>::CollectionInitiated(burn_id).into());
        })
    }

    #[test]
    fn collect_funds_should_return_error_when_already_collected() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);

            let attempt = CollectionInfo {
                status: types::CollectionStatus::Completed,
                reason: None,
            };
            Collections::<Test>::insert(burn_id.clone(), attempt);

            let expected_error = Error::<Test>::AlreadyCollected;
            let error_expression = Bridge::collect_funds(RuntimeOrigin::signed(1), burn_id.clone());

            assert_err!(error_expression, expected_error);
        })
    }

    #[test]
    fn collect_funds_cc2_should_return_error_when_already_in_progress() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);

            let progress = CollectionInfo {
                status: types::CollectionStatus::InProgress,
                reason: None,
            };
            InProgress::<Test>::insert(burn_id.clone(), progress);

            let expected_error = Error::<Test>::AlreadyInProgress;
            let error_expression = Bridge::collect_funds(RuntimeOrigin::signed(1), burn_id.clone());

            assert_err!(error_expression, expected_error);
        })
    }

    #[test]
    fn collect_funds_cc2_should_emit_event_when_moved_to_in_progress() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            System::assert_has_event(Event::<Test>::CollectionInitiated(burn_id).into());
        })
    }

    #[test]
    fn approve_collection_cc2_should_error_when_collection_not_found() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), collector));

            let expected_error = Error::<Test>::CollectionNotFound;

            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(collector), burn_id, collector, 0),
                expected_error
            );
        })
    }

    #[test]
    fn approve_collection_cc2_should_error_when_collection_completed() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let in_progress = CollectionInfo {
                status: types::CollectionStatus::Completed,
                reason: None,
            };
            InProgress::<Test>::insert(burn_id.clone(), in_progress);

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), collector));

            let expected_error = Error::<Test>::AlreadyCollected;
            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(collector), burn_id, collector, 0),
                expected_error
            );
        })
    }

    #[test]
    fn approve_collection_cc2_should_update_balance_when_successful() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let prior_balance = Balances::free_balance(collector);
            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), collector));

            assert_ok!(Bridge::approve_collection(
                RuntimeOrigin::signed(collector),
                burn_id,
                collector,
                100
            ),);

            let ending_balance = Balances::free_balance(collector);
            assert!(ending_balance > prior_balance);
        })
    }

    #[test]
    fn approve_collection_cc2_should_error_when_amount_is_invalid() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), collector));

            let expected_error = Error::<Test>::InvalidCollectionAmount;
            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(collector), burn_id, collector, 0),
                expected_error,
            );
        })
    }

    #[test]
    fn add_authority_should_work() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), collector));

            let authority = Bridge::authorities(collector);
            assert!(authority.is_some());
        })
    }

    #[test]
    fn add_authority_should_error_when_not_signed_by_root() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_err!(
                Bridge::add_authority(RuntimeOrigin::signed(1), collector),
                BadOrigin
            );
        })
    }

    #[test]
    fn remove_authority_should_error_when_not_signed_by_root() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_err!(
                Bridge::remove_authority(RuntimeOrigin::signed(1), collector),
                BadOrigin
            );
        })
    }

    #[test]
    fn add_authority_should_error_when_called_with_existing_authority() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_ok!(Bridge::add_authority(RuntimeOrigin::root(), collector));
            assert_err!(
                Bridge::add_authority(RuntimeOrigin::root(), collector),
                Error::<Test>::AlreadyAuthority
            );
        })
    }

    #[test]
    fn remove_authority_should_error_when_called_with_nonexisting_authority() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_err!(
                Bridge::remove_authority(RuntimeOrigin::root(), collector),
                Error::<Test>::NotAnAuthority
            );
        })
    }

    #[test]
    fn approve_collection_cc2_should_error_with_insufficient_authority() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            let expected_err = Error::<Test>::InsufficientAuthority;

            assert_err!(
                Bridge::approve_collection(
                    RuntimeOrigin::signed(collector),
                    burn_id,
                    collector,
                    100
                ),
                expected_err
            );
        })
    }

    #[test]
    fn reject_collection_cc2_should_error_with_insufficient_authority() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId::Creditcoin2(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            let expected_err = Error::<Test>::InsufficientAuthority;

            assert_err!(
                Bridge::reject_collection(
                    RuntimeOrigin::signed(collector),
                    burn_id,
                    types::FailureReason::BridgeError
                ),
                expected_err
            );
        })
    }
}
