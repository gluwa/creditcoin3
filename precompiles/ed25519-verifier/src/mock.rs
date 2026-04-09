use super::*;

use frame_support::{construct_runtime, parameter_types, traits::Everything, weights::Weight};
use pallet_evm::{
    AddressMapping, EnsureAddressNever, EnsureAddressRoot, FrameSystemAccountProvider,
    IdentityAddressMapping,
};
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::{H160, H256, U256};
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};

pub const PRECOMPILE_ADDRESS: u64 = 1;

pub type Balance = u128;
pub type AccountId = Account;
pub type Block = frame_system::mocking::MockBlockU32<Runtime>;

/// A simple account type.
#[cfg_attr(feature = "std", derive(serde::Serialize, serde::Deserialize))]
#[derive(
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Clone,
    Encode,
    Decode,
    Debug,
    MaxEncodedLen,
    derive_more::Display,
    TypeInfo,
)]
pub enum Account {
    Alice,
    Bob,
    Bogus,
    Precompile,
}

impl Default for Account {
    fn default() -> Self {
        Self::Bogus
    }
}

impl AddressMapping<Account> for Account {
    fn into_account_id(h160_account: H160) -> Account {
        match h160_account {
            a if a == H160::repeat_byte(0xAA) => Self::Alice,
            a if a == H160::repeat_byte(0xBB) => Self::Bob,
            a if a == H160::from_low_u64_be(PRECOMPILE_ADDRESS) => Self::Precompile,
            _ => Self::Bogus,
        }
    }
}

impl From<Account> for H160 {
    fn from(x: Account) -> H160 {
        match x {
            Account::Alice => H160::repeat_byte(0xAA),
            Account::Bob => H160::repeat_byte(0xBB),
            Account::Precompile => H160::from_low_u64_be(PRECOMPILE_ADDRESS),
            Account::Bogus => Default::default(),
        }
    }
}

impl From<H160> for Account {
    fn from(x: H160) -> Account {
        Account::into_account_id(x)
    }
}

parameter_types! {
    pub const BlockHashCount: u32 = 250;
    pub const SS58Prefix: u8 = 42;
}

impl frame_system::Config for Runtime {
    type PreInherents = ();
    type PostInherents = ();
    type PostTransactions = ();
    type RuntimeTask = RuntimeTask;
    type MultiBlockMigrator = ();
    type SingleBlockMigrations = ();
    type BaseCallFilter = Everything;
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type Nonce = u64;
    type Block = Block;
    type RuntimeCall = RuntimeCall;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type Lookup = IdentityLookup<Self::AccountId>;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = BlockHashCount;
    type Version = ();
    type PalletInfo = PalletInfo;
    type AccountData = pallet_balances::AccountData<Balance>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type BlockWeights = ();
    type BlockLength = ();
    type SS58Prefix = SS58Prefix;
    type OnSetCode = ();
    type MaxConsumers = frame_support::traits::ConstU32<16>;
}

parameter_types! {
    pub const MinimumPeriod: u64 = 5;
}

impl pallet_timestamp::Config for Runtime {
    type Moment = u64;
    type OnTimestampSet = ();
    type MinimumPeriod = MinimumPeriod;
    type WeightInfo = ();
}

parameter_types! {
    pub const ExistentialDeposit: u128 = 1;
}

impl pallet_balances::Config for Runtime {
    type MaxReserves = ();
    type ReserveIdentifier = ();
    type MaxLocks = ();
    type Balance = Balance;
    type RuntimeEvent = RuntimeEvent;
    type DustRemoval = ();
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = System;
    type WeightInfo = ();
    type RuntimeHoldReason = ();
    type FreezeIdentifier = ();
    type MaxFreezes = ();
    type RuntimeFreezeReason = RuntimeFreezeReason;
}

use precompile_utils::precompile_set::{AddressU64, PrecompileAt, PrecompileSetBuilder};

pub type Precompiles<R> =
    PrecompileSetBuilder<R, (PrecompileAt<AddressU64<1>, Ed25519VerifierPrecompile<R>>,)>;

pub type PCall = Ed25519VerifierPrecompileCall<Runtime>;

const MAX_POV_SIZE: u64 = 5 * 1024 * 1024;
/// Block storage limit in bytes. Set to 40 KB.
const BLOCK_STORAGE_LIMIT: u64 = 40 * 1024;

parameter_types! {
    pub BlockGasLimit: U256 = U256::from(u64::MAX);
    pub PrecompilesValue: Precompiles<Runtime> = Precompiles::new();
    pub const WeightPerGas: Weight = Weight::from_parts(1, 0);
    pub GasLimitPovSizeRatio: u64 = {
        let block_gas_limit = BlockGasLimit::get().min(u64::MAX.into()).low_u64();
        block_gas_limit.saturating_div(MAX_POV_SIZE)
    };
    pub GasLimitStorageGrowthRatio: u64 = {
        let block_gas_limit = BlockGasLimit::get().min(u64::MAX.into()).low_u64();
        block_gas_limit.saturating_div(BLOCK_STORAGE_LIMIT)
    };
    pub SuicideQuickClearLimit: u32 = 0;
}

impl pallet_evm::Config for Runtime {
    type FeeCalculator = ();
    type GasWeightMapping = pallet_evm::FixedGasWeightMapping<Self>;
    type WeightPerGas = WeightPerGas;
    type CallOrigin = EnsureAddressRoot<AccountId>;
    type WithdrawOrigin = EnsureAddressNever<AccountId>;
    type AddressMapping = IdentityAddressMapping;
    type Currency = Balances;
    type RuntimeEvent = RuntimeEvent;
    type Runner = pallet_evm::runner::stack::Runner<Self>;
    type PrecompilesType = Precompiles<Self>;
    type PrecompilesValue = PrecompilesValue;
    type ChainId = ();
    type OnChargeTransaction = ();
    type BlockGasLimit = BlockGasLimit;
    type BlockHashMapping = pallet_evm::SubstrateBlockHashMapping<Self>;
    type FindAuthor = ();
    type OnCreate = ();
    type GasLimitPovSizeRatio = GasLimitPovSizeRatio;
    type Timestamp = Timestamp;
    type WeightInfo = pallet_evm::weights::SubstrateWeight<Runtime>;
    type AccountProvider = FrameSystemAccountProvider<Runtime>;
    type GasLimitStorageGrowthRatio = GasLimitStorageGrowthRatio;
}

// Configure a mock runtime to test the precompile.
construct_runtime!(
    pub enum Runtime {
        System: frame_system,
        Balances: pallet_balances,
        Evm: pallet_evm,
        Timestamp: pallet_timestamp,
    }
);

#[derive(Default)]
pub(crate) struct ExtBuilder {}

impl ExtBuilder {
    pub(crate) fn build(self) -> sp_io::TestExternalities {
        let t = frame_system::GenesisConfig::<Runtime>::default()
            .build_storage()
            .expect("Frame system builds valid default genesis config");

        let mut ext = sp_io::TestExternalities::new(t);
        ext.execute_with(|| System::set_block_number(1));
        ext
    }
}
