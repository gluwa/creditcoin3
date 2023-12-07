#![cfg_attr(not(feature = "std"), no_std)]

mod migrations;
mod types;
use frame_support::traits::StorageVersion;
pub use pallet::*;

#[cfg(test)]
mod mock;

pub const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use crate::types::{BalanceFor, Cc2BurnId, CollectionInfo};
    use frame_support::dispatch::PostDispatchInfo;
    use frame_support::pallet_prelude::DispatchResult;
    use frame_support::traits::Currency;
    use frame_support::{pallet_prelude::*, Twox64Concat};
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{BlockNumberProvider, Zero};

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        type Currency: Currency<Self::AccountId>;
    }

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
        #[pallet::weight({0_0})]
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

        #[pallet::call_index(2)]
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

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_runtime_upgrade() -> Weight {
            migrations::migrate::<T>()
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
    use crate::{
        self as pallet_bridge,
        types::{Cc2BurnId, CollectionInfo},
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
    fn approve_collection_cc2_should_error_when_collection_completed() {
        new_test_ext().execute_with(|| {
            System::set_block_number(1);

            let burn_id = Cc2BurnId(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let completed = CollectionInfo {
                amount: 100,
                block_number: 1,
                collector,
            };
            Collections::<Test>::insert(&burn_id, completed);

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

            let burn_id = Cc2BurnId(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let prior_balance = Balances::free_balance(collector);

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

            let burn_id = Cc2BurnId(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

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

            let burn_id = Cc2BurnId(1);
            let collector = <Test as frame_system::Config>::AccountId::default();

            let authority = Bridge::authorities(collector);
            assert!(authority.is_none());

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
}
