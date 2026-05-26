use super::*;

use frame_election_provider_support::{
    bounds::{ElectionBounds, ElectionBoundsBuilder},
    onchain, SequentialPhragmen,
};
use frame_support::pallet_prelude::ConstU32;
use frame_support::traits::{AsEnsureOriginWithArg, ConstU128, ConstU64, KeyOwnerProofSystem};
use frame_support::{construct_runtime, parameter_types, traits::Everything, weights::Weight};
use pallet_babe::AuthorityId;
use pallet_evm::{
    EnsureAddressNever, EnsureAddressRoot, FrameSystemAccountProvider, HashedAddressMapping,
};
use pallet_session::historical as pallet_session_historical;
use pallet_staking::FixedNominationsQuota;
use sp_core::crypto::KeyTypeId;
use sp_core::{H160, H256, U256};
use sp_runtime::curve::PiecewiseLinear;
use sp_runtime::traits::OpaqueKeys;
use sp_runtime::Perbill;
use sp_runtime::{
    impl_opaque_keys,
    traits::{BlakeTwo256, IdentityLookup},
    AccountId32, BuildStorage,
};
use sp_staking::SessionIndex;
use supported_chains_primitives::MATURITY_FIXED_DELAY_10;

pub const PRECOMPILE_ADDRESS_U64: u64 = 1;

pub type Balance = u128;
pub type AccountId = AccountId32;
pub type Block = frame_system::mocking::MockBlockU32<Runtime>;

pub const SUPPORTED_CHAIN_KEY: u64 = 1;
pub const SUPPORTED_CHAIN_ID: u64 = 200;
pub const SUPPORTED_CHAIN_NAME: &[u8] = b"Ethereum";
pub const ERC20_ADDRESS: H160 = H160([
    0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01,
]);

/// Helper to create an AccountId32 from a u8 prefix (padded to 32 bytes).
#[allow(dead_code)]
pub fn account(prefix: u8) -> AccountId32 {
    AccountId32::from([prefix; 32])
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
    type MaxConsumers = ConstU32<16>;
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
    pub const MaxLocks: u32 = 50;
}

impl pallet_balances::Config for Runtime {
    type MaxReserves = ();
    type ReserveIdentifier = ();
    type MaxLocks = MaxLocks;
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

impl pallet_assets::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type Balance = Balance;
    type AssetId = u32;
    type AssetIdParameter = u32;
    type Currency = Balances;
    type CreateOrigin = AsEnsureOriginWithArg<frame_system::EnsureSigned<AccountId>>;
    type ForceOrigin = frame_system::EnsureRoot<AccountId>;
    type AssetDeposit = ConstU128<0>;
    type MetadataDepositBase = ConstU128<0>;
    type MetadataDepositPerByte = ConstU128<0>;
    type ApprovalDeposit = ConstU128<0>;
    type StringLimit = ConstU32<50>;
    type AssetAccountDeposit = ConstU128<0>;
    type RemoveItemsLimit = ConstU32<1000>;
    type Freezer = ();
    type Extra = ();
    type WeightInfo = ();
    type CallbackHandle = ();
}

use precompile_utils::precompile_set::{AddressU64, PrecompileAt, PrecompileSetBuilder};

pub type Precompiles<R> = PrecompileSetBuilder<
    R,
    (PrecompileAt<AddressU64<PRECOMPILE_ADDRESS_U64>, AttestCoinPrecompile<R>>,),
>;

const MAX_POV_SIZE: u64 = 5 * 1024 * 1024;
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
    type AddressMapping = HashedAddressMapping<BlakeTwo256>;
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
    pub const DefaultMaturityStrategy: &'static str = MATURITY_FIXED_DELAY_10;
}

pub struct DummyRegistrationHandler;
impl supported_chains_primitives::provider::OnRegisterChainProvider for DummyRegistrationHandler {
    fn on_register_chain(
        _chain_key: attestor_primitives::ChainKey,
        _chain_id: attestor_primitives::ChainId,
        _chain_name: sp_std::vec::Vec<u8>,
        _target_sample_size: Option<u32>,
        _chain_attestation_interval: Option<u64>,
        _attestation_checkpoint_interval: Option<u32>,
        _max_attestors: Option<u32>,
        _max_invulnerables: Option<u32>,
        _attestation_chain_genesis_block_number: Option<u64>,
        _encoding: attestor_primitives::ChainEncodingVersion,
    ) {
    }
}

impl pallet_supported_chains::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_supported_chains::weights::WeightInfo<Runtime>;
    type EventListeners = ();
    type ChainRegistrationHandler = DummyRegistrationHandler;
    type DefaultMaturityStrategy = DefaultMaturityStrategy;
    type OperatorsOrigin = frame_system::EnsureRoot<AccountId>;
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
    pub const SlashDeferDuration: sp_staking::EraIndex = 0;
    pub const RewardCurve: &'static PiecewiseLinear<'static> = &REWARD_CURVE;
    pub static ElectionsBounds: ElectionBounds = ElectionBoundsBuilder::default().build();
    pub const BondingDuration: sp_staking::EraIndex = 3;
}

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

impl pallet_session::historical::Config for Runtime {
    type FullIdentification = pallet_staking::Exposure<AccountId, u128>;
    type FullIdentificationOf = pallet_staking::ExposureOf<Self>;
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

parameter_types! {
    pub const MaxAttestorsDefault: u32 = 100;
    pub const CommittmentInterval: u64 = 1000;
    pub const DefaultAttestationsPerCheckpoint: u32 = 10;
    pub const DefaultAttestationInterval: u64 = 10;
    pub const DefaultTargetSampleSize: u32 = 3;
    pub const DefaultMinBondRequirement: u128 = 0;
    pub const MaxUnlockingChunks: u32 = 10;
    pub const MaxAttestationsPerBlock: u32 = 10;
    pub const DefaultAttestationChainGenesisBlockNumber: u64 = 0;
}

parameter_types! {
    /// Treasury account for holding attestation bond assets.
    pub const TreasuryAccount: AccountId32 = AccountId32::new([0xDDu8; 32]);
}

pub struct AttestationBondPoolAccount;
impl frame_support::traits::Get<AccountId32> for AttestationBondPoolAccount {
    fn get() -> AccountId32 {
        AccountId32::from([0xDD; 32])
    }
}

impl pallet_attestation::Config for Runtime {
    type DefaultAttestationsPerCheckpoint = DefaultAttestationsPerCheckpoint;
    type DefaultAttestationInterval = DefaultAttestationInterval;
    type DefaultTargetSampleSize = DefaultTargetSampleSize;
    type DefaultMaxCatchup = ConstU32<500>;
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_attestation::weights::WeightInfo<Runtime>;
    type MaxAttestationNodes = MaxAttestorsDefault;
    type CommittmentInterval = CommittmentInterval;
    type BlsSignature = [u8; 42];
    type SupportedChains = SupportedChains;
    type DefaultMinBondRequirement = DefaultMinBondRequirement;
    type NativeCurrency = Balances;
    type BondFungibles = Assets;
    type BondAssetId = ConstU32<1>;
    type BondPoolAccount = AttestationBondPoolAccount;
    type CurrencyBalance = Balance;
    type MaxUnlockingChunks = MaxUnlockingChunks;
    type BondingDuration = BondingDuration;
    type Staking = Staking;
    type Reward = ();
    type MaxAttestationsPerBlock = MaxAttestationsPerBlock;
    type DefaultAttestationRetentionDuration = ConstU32<10>;
    type MaxCheckpointsImportedPerCall = ConstU32<100>;
    type DefaultAttestationChainGenesisBlockNumber = DefaultAttestationChainGenesisBlockNumber;
    type OperatorsOrigin = frame_system::EnsureRoot<AccountId>;
    type CommittedAttestationHook = pallet_attestation::NoopCommittedAttestationObserver;
}

impl pallet_attest_coin_rewards::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type RewardPoints = u128;
    type RewardPerEligibleSigner = ConstU128<100>;
    type WeightInfo = pallet_attest_coin_rewards::weights::WeightInfo<Runtime>;
}

construct_runtime!(
    pub enum Runtime {
        System: frame_system,
        Balances: pallet_balances,
        Assets: pallet_assets,
        Evm: pallet_evm,
        Timestamp: pallet_timestamp,
        SupportedChains: pallet_supported_chains,
        Attestation: pallet_attestation,
        AttestCoinRewards: pallet_attest_coin_rewards,
        Staking: pallet_staking,
        Session: pallet_session,
        Babe: pallet_babe,
        Historical: pallet_session_historical,
    }
);

pub fn alice() -> AccountId32 {
    AccountId32::from([0xAAu8; 32])
}

pub fn bob() -> AccountId32 {
    AccountId32::from([0xBBu8; 32])
}

pub fn treasury() -> AccountId32 {
    AccountId32::from([0xDDu8; 32])
}

#[derive(Default)]
pub(crate) struct ExtBuilder {
    extra_balances: Vec<(AccountId, Balance)>,
}

impl ExtBuilder {
    #[allow(dead_code)]
    pub(crate) fn with_balances(mut self, balances: Vec<(AccountId, Balance)>) -> Self {
        self.extra_balances = balances;
        self
    }

    pub(crate) fn build(self) -> sp_io::TestExternalities {
        let _ = env_logger::try_init();

        let mut t = frame_system::GenesisConfig::<Runtime>::default()
            .build_storage()
            .expect("Frame system builds valid default genesis config");

        let attest_coin: Balance = 10_000_000_000_000_000_000_000;
        let mut balances = vec![
            (alice(), attest_coin),
            (bob(), attest_coin),
            (treasury(), attest_coin),
        ];
        balances.extend(self.extra_balances);

        pallet_balances::GenesisConfig::<Runtime> { balances }
            .assimilate_storage(&mut t)
            .expect("Pallet balances storage can be assimilated");

        pallet_assets::GenesisConfig::<Runtime> {
            assets: vec![(1, alice(), false, 1)],
            metadata: vec![(1, b"AC".to_vec(), b"AC".to_vec(), 18)],
            accounts: vec![(1, alice(), attest_coin), (1, bob(), attest_coin)],
            next_asset_id: Some(2),
        }
        .assimilate_storage(&mut t)
        .expect("Pallet assets genesis");

        pallet_supported_chains::GenesisConfig::<Runtime> {
            supported_chains: vec![(
                SUPPORTED_CHAIN_ID,
                SUPPORTED_CHAIN_NAME.to_vec(),
                attestor_primitives::ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            )],
            _phantom: Default::default(),
        }
        .assimilate_storage(&mut t)
        .unwrap();

        let mut ext = sp_io::TestExternalities::new(t);
        ext.execute_with(|| System::set_block_number(1));
        ext
    }
}
