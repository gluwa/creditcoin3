use super::*;

use frame_election_provider_support::{
    bounds::{ElectionBounds, ElectionBoundsBuilder},
    onchain, SequentialPhragmen,
};
use frame_support::pallet_prelude::ConstU32;
use frame_support::traits::{ConstU64, KeyOwnerProofSystem};
use frame_support::{construct_runtime, parameter_types, traits::Everything, weights::Weight};
use pallet_babe::AuthorityId;
use pallet_evm::{
    EnsureAddressNever, EnsureAddressRoot, FrameSystemAccountProvider, IdentityAddressMapping,
};
use pallet_session::historical as pallet_session_historical;
use pallet_session::Config;
use pallet_staking::FixedNominationsQuota;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::crypto::KeyTypeId;
use sp_core::{H160, H256, U256};
use sp_runtime::curve::PiecewiseLinear;
use sp_runtime::traits::OpaqueKeys;
use sp_runtime::Perbill;
use sp_runtime::{
    impl_opaque_keys,
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};
use sp_staking::SessionIndex;

pub const PRECOMPILE_ADDRESS: u64 = 1;

pub type Balance = u128;
pub type AccountId = Account;
pub type Block = frame_system::mocking::MockBlockU32<Runtime>;

/// A simple account type.
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
    Serialize,
    Deserialize,
    derive_more::Display,
    TypeInfo,
)]
pub enum Account {
    Alice,
    Bob,
    Charlie,
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
            a if a == H160::repeat_byte(0xCC) => Self::Charlie,
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
            Account::Charlie => H160::repeat_byte(0xCC),
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

impl From<Account> for H256 {
    fn from(x: Account) -> H256 {
        let x: H160 = x.into();
        x.into()
    }
}

type BytesArray = [u8; 32];

impl From<BytesArray> for Account {
    fn from(value: BytesArray) -> Self {
        let h = H256::from(value); // Convert BytesArray directly into H256
        let addr = H160::from(h); // Convert H256 into H160
        Account::from(addr) // Convert H160 into Account
    }
}

impl From<sp_core::sr25519::Public> for Account {
    fn from(value: sp_core::sr25519::Public) -> Self {
        let h = H256::from(value); // Convert BytesArray directly into H256
        let addr = H160::from(h); // Convert H256 into H160
        Account::from(addr)
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
    PrecompileSetBuilder<R, (PrecompileAt<AddressU64<1>, ProofVerifierPrecompile<R>>,)>;

pub type PCall = ProofVerifierPrecompileCall<Runtime>;

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

parameter_types! {
    pub const MaxSegmentsPerVerifierResult: u32 = 1000;
}

impl pallet_prover::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_prover::weights::WeightInfo<Runtime>;
    type SupportedChains = SupportedChains;
    type Checkpoints = Attestation;
    type Attestations = Attestation;
    type MaxSegmentsPerVerifierResult = MaxSegmentsPerVerifierResult;
}

impl pallet_supported_chains::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_supported_chains::weights::WeightInfo<Runtime>;
    type EventListeners = ();
}
pub const SLASHING_DISABLING_FACTOR: usize = 3;

pallet_staking_reward_curve::build! {
    const REWARD_CURVE: PiecewiseLinear<'static> = curve!(
        min_inflation: 0_025_000u64,
        max_inflation: 0_100_000,
        ideal_stake: 0_500_000,
        falloff: 0_050_000,
        max_piece_count: 40,
        test_precision: 0_005_000,
    );
}
parameter_types! {
    pub const SessionsPerEra: SessionIndex = 3;
    pub const SlashDeferDuration: EraIndex = 0;
    pub const RewardCurve: &'static PiecewiseLinear<'static> = &REWARD_CURVE;
    pub const OffendingValidatorsThreshold: Perbill = Perbill::from_percent(16);
    pub static ElectionsBounds: ElectionBounds = ElectionBoundsBuilder::default().build();
}
type DummyValidatorId = u64;

pub struct OnChainSeqPhragmen;
impl onchain::Config for OnChainSeqPhragmen {
    type System = Runtime;
    type Solver = SequentialPhragmen<AccountId, Perbill>;
    type DataProvider = Staking;
    type WeightInfo = ();
    type MaxWinners = ConstU32<100>;
    type Bounds = ElectionsBounds;
}

impl pallet_staking::Config for Runtime {
    type RewardRemainder = ();
    type CurrencyToVote = ();
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
    type CurrencyBalance = <Self as pallet_balances::Config>::Balance;
    type Slash = ();
    type Reward = ();
    type SessionsPerEra = SessionsPerEra;
    type BondingDuration = BondingDuration;
    type SlashDeferDuration = SlashDeferDuration;
    type AdminOrigin = frame_system::EnsureRoot<Self::AccountId>;
    type SessionInterface = Self;
    type UnixTime = pallet_timestamp::Pallet<Runtime>;
    type EraPayout = pallet_staking::ConvertCurve<RewardCurve>;
    type NextNewSession = Session;
    type MaxExposurePageSize = ConstU32<256>;
    type ElectionProvider = onchain::OnChainExecution<OnChainSeqPhragmen>;
    type GenesisElectionProvider = Self::ElectionProvider;
    type VoterList = pallet_staking::UseNominatorsAndValidatorsMap<Self>;
    type TargetList = pallet_staking::UseValidatorsMap<Self>;
    type NominationsQuota = FixedNominationsQuota<16>;
    type MaxUnlockingChunks = ConstU32<32>;
    type HistoryDepth = ConstU32<84>;
    type EventListeners = ();
    type BenchmarkingConfig = pallet_staking::TestBenchmarkingConfig;
    type WeightInfo = ();
    type MaxControllersInDeprecationBatch = ConstU32<100>;
    type DisablingStrategy = pallet_staking::UpToLimitDisablingStrategy<SLASHING_DISABLING_FACTOR>;
}

impl_opaque_keys! {
    pub struct MockSessionKeys {
        pub babe_authority: pallet_babe::Pallet<Runtime>,
    }
}

impl pallet_session::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type ValidatorId = <Self as frame_system::Config>::AccountId;
    type ValidatorIdOf = pallet_staking::StashOf<Self>;
    type ShouldEndSession = Babe;
    type NextSessionRotation = Babe;
    type SessionManager = pallet_session::historical::NoteHistoricalRoot<Self, Staking>;
    type SessionHandler = <MockSessionKeys as OpaqueKeys>::KeyTypeIdProviders;
    type Keys = MockSessionKeys;
    type WeightInfo = ();
}

parameter_types! {
    pub const EpochDuration: u64 = 3;
    pub const ReportLongevity: u64 =
        BondingDuration::get() as u64 * SessionsPerEra::get() as u64 * EpochDuration::get();
}

impl pallet_babe::Config for Runtime {
    type EpochDuration = EpochDuration;
    type ExpectedBlockTime = ConstU64<1>;
    type EpochChangeTrigger = pallet_babe::ExternalTrigger;
    type DisabledValidators = Session;
    type WeightInfo = ();
    type MaxAuthorities = ConstU32<10>;
    type MaxNominators = ConstU32<100>;
    type KeyOwnerProof = <Historical as KeyOwnerProofSystem<(KeyTypeId, AuthorityId)>>::Proof;
    type EquivocationReportSystem = ();
}

impl pallet_session::historical::Config for Runtime {
    type FullIdentification = pallet_staking::Exposure<Account, u128>;
    type FullIdentificationOf = pallet_staking::ExposureOf<Self>;
}

use sp_staking::EraIndex;

parameter_types! {
    pub const MaxLocks: u32 = 50;
    pub const MaxAttestorsDefault: u32 = 100;
    pub const CommittmentInterval: u64 = 1000;
    pub const DefaultAttestationsPerCheckpoint: u32 = 10;
    pub const DefaultAttestationInterval: u64 = 10;
    pub const DefaultTargetSampleSize: u32 = 3;
    pub const DefaultMinBondRequirement: u64 = 10_000;
    pub const MaxUnlockingChunks: u32 = 10;
    pub const MaxAttestationsPerBlock: u32 = 10;
    pub const BondingDuration: EraIndex = 3;
}

impl pallet_attestation_poc::Config for Runtime {
    type DefaultAttestationsPerCheckpoint = DefaultAttestationsPerCheckpoint;
    type DefaultAttestationInterval = DefaultAttestationInterval;
    type DefaultTargetSampleSize = DefaultTargetSampleSize;
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_attestation_poc::weights::WeightInfo<Runtime>;
    type MaxAttestationNodes = MaxAttestorsDefault;
    type CommittmentInterval = CommittmentInterval;
    type BlsSignature = [u8; 42];
    type SupportedChains = SupportedChains;
    type DefaultMinBondRequirement = DefaultMinBondRequirement;
    type Currency = Balances;
    type CurrencyBalance = Balance;
    type MaxUnlockingChunks = MaxUnlockingChunks;
    type BondingDuration = BondingDuration;
    type Staking = Staking;
    type Reward = ();
    type MaxAttestationsPerBlock = MaxAttestationsPerBlock;
}

// Configure a mock runtime to test the pallet.
construct_runtime!(
    pub enum Runtime {
        System: frame_system,
        Balances: pallet_balances,
        Evm: pallet_evm,
        Timestamp: pallet_timestamp,
        ProverModule: pallet_prover,
        SupportedChains: pallet_supported_chains,
        Attestation: pallet_attestation_poc,
        Staking: pallet_staking,
        Session: pallet_session,
        Babe: pallet_babe,
        Historical: pallet_session_historical,
    }
);

#[derive(Default)]
pub(crate) struct ExtBuilder {
    // endowed accounts with balances
    balances: Vec<(AccountId, Balance)>,
}

impl ExtBuilder {
    pub(crate) fn with_balances(mut self, balances: Vec<(AccountId, Balance)>) -> Self {
        self.balances = balances;
        self
    }

    pub(crate) fn build(self) -> sp_io::TestExternalities {
        // Init env logger to see logs in debug mode
        let _ = env_logger::try_init();

        let mut t = frame_system::GenesisConfig::<Runtime>::default()
            .build_storage()
            .expect("Frame system builds valid default genesis config");

        pallet_balances::GenesisConfig::<Runtime> {
            balances: self.balances,
        }
        .assimilate_storage(&mut t)
        .expect("Pallet balances storage can be assimilated");

        let chains = pallet_supported_chains::GenesisConfig::<Runtime> {
            supported_chains: vec![(1, "Ethereum".as_bytes().to_vec())],
            _phantom: Default::default(),
        };
        chains.assimilate_storage(&mut t).unwrap();

        let mut ext = sp_io::TestExternalities::new(t);
        ext.execute_with(|| System::set_block_number(1));
        ext
    }
}
