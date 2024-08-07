use crate as randomness_pallet;
use frame_support::traits::{ConstU16, ConstU64, ConstU32};
use sp_core::H256;
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};

// //todo need to add babe pallet config for Test runtime
// type Block = frame_system::mocking::MockBlock<Test>;

// frame_support::construct_runtime!(
//     pub enum Test
//     {
//         System: frame_system,
//         // SupportedChain: randomness_pallet,
//         Babe: pallet_babe,
//         Timestamp: pallet_timestamp,
//         RandomnessPallet: randomness_pallet,
//     }
// );

// impl frame_system::Config for Test {
//     type BaseCallFilter = frame_support::traits::Everything;
//     type BlockWeights = ();
//     type BlockLength = ();
//     type DbWeight = ();
//     type RuntimeOrigin = RuntimeOrigin;
//     type RuntimeCall = RuntimeCall;
//     type Nonce = u64;
//     type Hash = H256;
//     type Hashing = BlakeTwo256;
//     type AccountId = u64;
//     type Lookup = IdentityLookup<Self::AccountId>;
//     type Block = Block;
//     type RuntimeEvent = RuntimeEvent;
//     type BlockHashCount = ConstU64<250>;
//     type Version = ();
//     type PalletInfo = PalletInfo;
//     type AccountData = ();
//     type OnNewAccount = ();
//     type OnKilledAccount = ();
//     type SystemWeightInfo = ();
//     type SS58Prefix = ConstU16<42>;
//     type OnSetCode = ();
//     type MaxConsumers = frame_support::traits::ConstU32<16>;
// }


// use frame_support::parameter_types;
// parameter_types! {
//     pub const EpochDuration: u64 = 3;
// }   

// impl pallet_timestamp::Config for Test {
// 	type Moment = u64;
// 	type OnTimestampSet = Babe;
// 	type MinimumPeriod = ConstU64<1>;
// 	type WeightInfo = ();
// }

// impl pallet_babe::Config for Test {
//     type EpochDuration = EpochDuration;
//     type ExpectedBlockTime = ConstU64<1>;
//     type EpochChangeTrigger = pallet_babe::ExternalTrigger;
//     type KeyOwnerProof = sp_session::MembershipProof;
//     type EquivocationReportSystem =();
//     type WeightInfo = ();
//     type MaxAuthorities = ConstU32<10>;
// 	type MaxNominators = ConstU32<100>;
//     type DisabledValidators = ();
// }

// impl randomness_pallet::Config for Test {
//     type RuntimeEvent = RuntimeEvent;
//     type WeightInfo = randomness_pallet::weights::WeightInfo<Test>;
// }


// #[derive(Default)]
// pub struct ExtBuilder;

// impl ExtBuilder {
//     pub fn with_empty(self) -> sp_io::TestExternalities {
//         let mut storage = frame_system::GenesisConfig::<Test>::default()
//             .build_storage()
//             .unwrap();

//         storage.into()
//     }

//     pub fn build_and_execute(self, test: impl FnOnce()) {
//         self.with_empty().execute_with(test);
//     }
// }

// pub fn new_test_ext() -> sp_io::TestExternalities {
//     frame_system::GenesisConfig::<Test>::default()
//         .build_storage()
//         .unwrap()
//         .into()
// }
