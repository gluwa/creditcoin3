use crate::{self as prover_pallet};
use frame_election_provider_support::{
    bounds::{ElectionBounds, ElectionBoundsBuilder},
    onchain, SequentialPhragmen,
};
use frame_support::traits::{ConstU64, KeyOwnerProofSystem};
use frame_support::{parameter_types, traits::ConstU32};
use frame_system as system;
use pallet_babe::AuthorityId;
use pallet_session::historical as pallet_session_historical;
use sp_core::H256;
use sp_runtime::{
    impl_opaque_keys,
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};
use sp_staking::EraIndex;

type AccountId = u64;
type Balance = u128;
type Block = frame_system::mocking::MockBlock<Test>;
type Nonce = u32;

// Configure a mock runtime to test the pallet.
frame_support::construct_runtime!(
    pub enum Test {
        System: frame_system,
        Balances: pallet_balances,
        ProverModule: prover_pallet,
        Historical: pallet_session_historical,
        SupportedChains: pallet_supported_chains,
        Attestation: pallet_attestation_poc,
        Staking: pallet_staking,
        Timestamp: pallet_timestamp,
        Babe: pallet_babe,
        Session: pallet_session,
    }
);

impl_opaque_keys! {
    pub struct MockSessionKeys {
        pub babe_authority: pallet_babe::Pallet<Test>,
    }
}

impl pallet_session::Config for Test {
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
    pub const BlockHashCount: u64 = 250;
    pub const SS58Prefix: u8 = 42;
}

impl frame_system::Config for Test {
    type PreInherents = ();
    type PostInherents = ();
    type PostTransactions = ();
    type RuntimeTask = RuntimeTask;
    type MultiBlockMigrator = ();
    type SingleBlockMigrations = ();
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
    type MaxFreezes = ();
    type RuntimeFreezeReason = RuntimeFreezeReason;
}

parameter_types! {
    pub const MaxSegmentsPerVerifierResult: u32 = 1000;
}

impl prover_pallet::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = prover_pallet::weights::WeightInfo<Test>;
    type SupportedChains = SupportedChains;
    type Checkpoints = Attestation;
    type Attestations = Attestation;
    type MaxSegmentsPerVerifierResult = MaxSegmentsPerVerifierResult;
}

parameter_types! {
    pub const EpochDuration: u64 = 3;
    pub const ReportLongevity: u64 =
        BondingDuration::get() as u64 * SessionsPerEra::get() as u64 * EpochDuration::get();
}

impl pallet_babe::Config for Test {
    type EpochDuration = EpochDuration;
    type ExpectedBlockTime = ConstU64<1>;
    type EpochChangeTrigger = pallet_babe::ExternalTrigger;
    type DisabledValidators = (); //Session;
    type WeightInfo = ();
    type MaxAuthorities = ConstU32<10>;
    type MaxNominators = ConstU32<100>;
    type KeyOwnerProof = <Historical as KeyOwnerProofSystem<(KeyTypeId, AuthorityId)>>::Proof;
    type EquivocationReportSystem = ();
}

impl pallet_session::historical::Config for Test {
    type FullIdentification = pallet_staking::Exposure<u64, u128>;
    type FullIdentificationOf = pallet_staking::ExposureOf<Self>;
}

impl pallet_timestamp::Config for Test {
    type Moment = u64;
    type OnTimestampSet = Babe;
    type MinimumPeriod = ConstU64<1>;
    type WeightInfo = ();
}

impl pallet_supported_chains::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_supported_chains::weights::WeightInfo<Test>;
    type EventListeners = ();
}

parameter_types! {
    pub const DefaultAttestationsPerCheckpoint: u32 = 10;
    pub const DefaultAttestationInterval: u64 = 10;
    pub const DefaultTargetSampleSize: u32 = 3;
    pub const DefaultMinBondRequirement: u64 = 10_000;
    pub const MaxUnlockingChunks: u32 = 10;
    pub const MaxAttestationsPerBlock: u32 = 10;
    pub const BondingDuration: EraIndex = 3;
}

impl pallet_attestation_poc::Config for Test {
    type DefaultAttestationsPerCheckpoint = DefaultAttestationsPerCheckpoint;
    type DefaultAttestationInterval = DefaultAttestationInterval;
    type DefaultTargetSampleSize = DefaultTargetSampleSize;
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_attestation_poc::weights::WeightInfo<Test>;
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

use sp_runtime::curve::PiecewiseLinear;
use sp_runtime::Perbill;
use sp_staking::SessionIndex;

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

pub const SLASHING_DISABLING_FACTOR: usize = 3;
use pallet_staking::FixedNominationsQuota;
use sp_core::crypto::KeyTypeId;
use sp_runtime::traits::OpaqueKeys;

type DummyValidatorId = u64;

pub struct OnChainSeqPhragmen;
impl onchain::Config for OnChainSeqPhragmen {
    type System = Test;
    type Solver = SequentialPhragmen<DummyValidatorId, Perbill>;
    type DataProvider = Staking;
    type WeightInfo = ();
    type MaxWinners = ConstU32<100>;
    type Bounds = ElectionsBounds;
}

impl pallet_staking::Config for Test {
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
    type UnixTime = pallet_timestamp::Pallet<Test>;
    type EraPayout = pallet_staking::ConvertCurve<RewardCurve>;
    type NextNewSession = (); //Session;
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

// add more accounts when you need them
// and update balances genesis below
pub(crate) const PROVER_1: AccountId = 1;
pub(crate) const PROVER_2: AccountId = 2;
pub(crate) const CLAIMER_1: AccountId = 3;
pub(crate) const CLAIMER_2: AccountId = 4;
pub(crate) const PROVER_3: AccountId = 5;

#[derive(Default)]
pub struct ExtBuilder;

impl ExtBuilder {
    pub fn build(self) -> sp_io::TestExternalities {
        // Init env logger to see logs in debug mode
        let _ = env_logger::try_init();

        let mut t = system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();
        // accounts 0 to 5 have initial balances
        let b = pallet_balances::GenesisConfig::<Test> {
            balances: vec![
                (PROVER_1, 9_000_000_000_000_000_000),
                (PROVER_2, 50_000_000_000_000_000_000),
                (CLAIMER_1, 50_000_000_000_000_000_000),
                (CLAIMER_2, 50_000_000_000_000_000_000),
                (PROVER_3, 50_000_000_000_000_000_000),
            ],
        };
        b.assimilate_storage(&mut t).unwrap();

        let chains = pallet_supported_chains::GenesisConfig::<Test> {
            supported_chains: vec![(1, "Ethereum".as_bytes().to_vec())],
            _phantom: Default::default(),
        };
        chains.assimilate_storage(&mut t).unwrap();

        // let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
        // };
        // pallet_genesis.assimilate_storage(&mut t).unwrap();

        t.into()
    }

    pub fn build_and_execute<R>(self, test: impl FnOnce() -> R) -> R {
        self.build().execute_with(test)
    }
}
