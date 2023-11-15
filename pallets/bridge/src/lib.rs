#![cfg_attr(not(feature = "std"), no_std)]

mod types;

#[frame_support::pallet]
pub mod pallet {
    use crate::types::{BurnId, CollectionInfo, CollectionStatus};
    use frame_support::pallet_prelude::DispatchResult;
    use frame_support::traits::{fungible::Mutate, ReservableCurrency};
    use frame_support::{
        pallet_prelude::{OptionQuery, *},
        Twox64Concat,
    };
    use frame_system::pallet_prelude::*;

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
        TestEvent,
        CollectionInitiated,
        CollectionInProgress,
        FundsCollected,
        CollectionFailed,
        CollectionExpired,
    }

    #[pallet::error]
    pub enum Error<T> {
        AlreadyCollected,
        AlreadyInProgress,
        InProgressCollectionNotFound,
        CollectionNotInProgress,
    }

    #[pallet::storage]
    #[pallet::getter(fn collections)]
    pub(super) type Collections<T: Config> =
        StorageMap<_, Twox64Concat, BurnId, CollectionInfo, OptionQuery>;

    #[pallet::storage]
    #[pallet::getter(fn in_progress)]
    pub(super) type InProgress<T: Config> =
        StorageMap<_, Twox64Concat, BurnId, CollectionInfo, OptionQuery>;

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight({0_0})]
        pub fn emit_test_event(origin: OriginFor<T>) -> DispatchResult {
            let _ = ensure_signed(origin)?;
            Self::deposit_event(Event::<T>::TestEvent);
            Ok(())
        }

        #[pallet::call_index(1)]
        #[pallet::weight({0_0})]
        pub fn collect_funds(origin: OriginFor<T>, burn_id: BurnId) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let collection_attempt = Self::collections(burn_id.clone());

            if collection_attempt.is_none() {
                let new_attempt: CollectionInfo = Default::default();
                Collections::<T>::insert(burn_id.clone(), new_attempt);
                Self::deposit_event(Event::<T>::CollectionInitiated);
                return Ok(());
            }

            let collection_attempt = collection_attempt.expect("This should never fail");

            ensure!(
                collection_attempt.status != CollectionStatus::Completed,
                Error::<T>::AlreadyCollected
            );

            let in_progress = InProgress::<T>::get(burn_id.clone());

            ensure!(in_progress.is_none(), Error::<T>::AlreadyInProgress);

            let in_progress = CollectionInfo {
                status: CollectionStatus::InProgress,
            };
            InProgress::<T>::insert(burn_id.clone(), in_progress);

            let collection_attempt = CollectionInfo {
                status: CollectionStatus::InProgress,
            };
            Collections::<T>::insert(burn_id.clone(), collection_attempt);

            Self::deposit_event(Event::<T>::CollectionInProgress);
            Ok(())
        }

        #[pallet::call_index(2)]
        #[pallet::weight({0_0})]
        pub fn approve_collection(
            origin: OriginFor<T>,
            burn_id: BurnId,
            collector: T::AccountId,
            amount: T::Balance,
        ) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let in_progress = InProgress::<T>::get(burn_id.clone());

            ensure!(
                in_progress.is_some(),
                Error::<T>::InProgressCollectionNotFound
            );

            let in_progress = in_progress.expect("This should never fail");

            ensure!(
                in_progress.status != CollectionStatus::Completed,
                Error::<T>::AlreadyCollected
            );

            <pallet_balances::Pallet<T> as Mutate<T::AccountId>>::mint_into(&collector, amount)?;

            let status = CollectionInfo {
                status: CollectionStatus::Completed,
            };

            InProgress::<T>::remove(burn_id.clone());
            Collections::<T>::insert(burn_id, status);

            Self::deposit_event(Event::<T>::FundsCollected);
            Ok(())
        }

        #[pallet::call_index(3)]
        #[pallet::weight({0_0})]
        pub fn reject_collection(origin: OriginFor<T>, burn_id: BurnId) -> DispatchResult {
            let _ = ensure_signed(origin)?;

            let in_progress = InProgress::<T>::get(burn_id.clone());

            ensure!(
                in_progress.is_some(),
                Error::<T>::InProgressCollectionNotFound
            );

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
            };

            InProgress::<T>::remove(burn_id.clone());
            Collections::<T>::insert(burn_id.clone(), status);

            Self::deposit_event(Event::<T>::CollectionFailed);
            Ok(())
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
        traits::{BlakeTwo256, IdentityLookup},
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
        type RuntimeOrigin = Self::RuntimeOrigin;
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
    fn test_emit_event() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let _ = Bridge::emit_test_event(RuntimeOrigin::signed(1));

            System::assert_has_event(Event::<Test>::TestEvent.into());
        });
    }

    #[test]
    fn collect_funds_should_create_new_collection_when_burn_id_nonexisting() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId(1);

            let existing_attempt = Collections::<Test>::get(burn_id.clone());
            assert!(existing_attempt.is_none());

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            let existing_attempt = Collections::<Test>::get(burn_id.clone());
            assert!(existing_attempt.is_some());

            System::assert_has_event(Event::<Test>::CollectionInitiated.into());
        })
    }

    #[test]
    fn collect_funds_should_return_error_when_already_collected() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId(1);

            let attempt = CollectionInfo {
                status: types::CollectionStatus::Completed,
            };
            Collections::<Test>::insert(burn_id.clone(), attempt);

            let expected_error = Error::<Test>::AlreadyCollected;
            let error_expression = Bridge::collect_funds(RuntimeOrigin::signed(1), burn_id.clone());

            assert_err!(error_expression, expected_error);
        })
    }

    #[test]
    fn collect_funds_should_return_error_when_already_in_progress() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId(1);

            let attempt = CollectionInfo {
                status: types::CollectionStatus::NotStarted,
            };
            Collections::<Test>::insert(burn_id.clone(), attempt);

            let progress = CollectionInfo {
                status: types::CollectionStatus::InProgress,
            };
            InProgress::<Test>::insert(burn_id.clone(), progress);

            let expected_error = Error::<Test>::AlreadyInProgress;
            let error_expression = Bridge::collect_funds(RuntimeOrigin::signed(1), burn_id.clone());

            assert_err!(error_expression, expected_error);
        })
    }

    #[test]
    fn collect_funds_should_emit_event_when_moved_to_in_progress() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId(1);

            let attempt = CollectionInfo {
                status: types::CollectionStatus::NotStarted,
            };
            Collections::<Test>::insert(burn_id.clone(), attempt);

            assert_ok!(Bridge::collect_funds(
                RuntimeOrigin::signed(1),
                burn_id.clone()
            ));

            System::assert_has_event(Event::<Test>::CollectionInProgress.into());
        })
    }

    #[test]
    fn approve_collection_should_error_when_collection_not_found() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let expected_error = Error::<Test>::InProgressCollectionNotFound;
            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(1), burn_id, collector, 0),
                expected_error
            );
        })
    }

    #[test]
    fn approve_collection_should_error_when_collection_completed() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = BurnId(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let in_progress = CollectionInfo {
                status: types::CollectionStatus::Completed,
            };
            InProgress::<Test>::insert(burn_id.clone(), in_progress);

            let expected_error = Error::<Test>::AlreadyCollected;
            assert_err!(
                Bridge::approve_collection(RuntimeOrigin::signed(1), burn_id, collector, 0),
                expected_error
            );
        })
    }
}
