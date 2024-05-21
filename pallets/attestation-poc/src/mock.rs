use crate::{self as attestation_poc, tests::VALID_BLS_PUBLIC_KEY};
use attestor_primitives::BlsPublicKeyWrapper;
use frame_support::{parameter_types, traits::ConstU32};
use frame_system as system;
use sp_core::H256;
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};

type AccountId = u64;
type Balance = u128;
type Block = frame_system::mocking::MockBlock<Test>;
type Nonce = u32;

// Configure a mock runtime to test the pallet.
frame_support::construct_runtime!(
    pub enum Test {
        System: frame_system,
        Balances: pallet_balances,
        Attestation: attestation_poc,
        SupportedChains: pallet_supported_chains,
    }
);

parameter_types! {
    pub const BlockHashCount: u64 = 250;
    pub const SS58Prefix: u8 = 42;
}

impl frame_system::Config for Test {
    type BaseCallFilter = frame_support::traits::Everything;
    type Block = Block;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type Nonce = Nonce;
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type AccountData = pallet_balances::AccountData<Balance>;
    type Lookup = IdentityLookup<Self::AccountId>;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = BlockHashCount;
    type Version = ();
    type PalletInfo = PalletInfo;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type SS58Prefix = SS58Prefix;
    type OnSetCode = ();
    type MaxConsumers = ConstU32<{ u32::MAX }>;
}

parameter_types! {
    pub const ExistentialDeposit: u128 = 500;
    pub const MaxLocks: u32 = 50;
    pub const MaxAttestorsDefault:u32 = 100;
    pub const CommittmentInterval: u64 = 1000;
}

impl pallet_balances::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_balances::weights::SubstrateWeight<Self>;
    type Balance = Balance;
    type DustRemoval = ();
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = System;
    type ReserveIdentifier = [u8; 8];
    type RuntimeHoldReason = ();
    type FreezeIdentifier = ();
    type MaxLocks = MaxLocks;
    type MaxReserves = ();
    type MaxHolds = ();
    type MaxFreezes = ();
}

impl attestation_poc::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = attestation_poc::weights::WeightInfo<Test>;
    type MaxAttestationNodes = MaxAttestorsDefault;
    type CommittmentInterval = CommittmentInterval;
    type BlsSignature = [u8; 42];
    type SupportedChains = SupportedChains;
}

impl pallet_supported_chains::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_supported_chains::weights::WeightInfo<Test>;
}

// add more accounts when you need them
// and update balances genesis below
pub(crate) const ATTESTOR_1: AccountId = 1;
pub(crate) const ATTESTOR_2: AccountId = 2;
pub(crate) const ATTESTOR_3: AccountId = 3;

pub(crate) const DEFAULT_COMITTEE_SET_SIZE: u32 = 250;

#[derive(Default)]
pub struct ExtBuilder;

impl ExtBuilder {
    pub fn build(self) -> sp_io::TestExternalities {
        let mut t = system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();
        // accounts 0 to 5 have initial balances
        let b = pallet_balances::GenesisConfig::<Test> {
            balances: vec![
                (ATTESTOR_1, 9_000_000_000_000_000_000),
                (ATTESTOR_2, 50_000_000_000_000_000_000),
            ],
        };
        b.assimilate_storage(&mut t).unwrap();

        let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
            comittee_set_size: DEFAULT_COMITTEE_SET_SIZE,
            invulnerables: vec![(ATTESTOR_3, BlsPublicKeyWrapper(VALID_BLS_PUBLIC_KEY))],
            supported_chains: vec![(1, 10)],
        };
        pallet_genesis.assimilate_storage(&mut t).unwrap();

        t.into()
    }

    pub fn build_and_execute<R>(self, test: impl FnOnce() -> R) -> R {
        self.build().execute_with(test)
    }
}
