use crate::{self as attestation_poc, tests::Attestor, TargetSampleSizeDefault};
use frame_election_provider_support::{
    bounds::{ElectionBounds, ElectionBoundsBuilder},
    onchain, SequentialPhragmen,
};
use frame_support::{
    parameter_types,
    traits::{ConstU32, ConstU64, KeyOwnerProofSystem, OnInitialize},
};
use pallet_babe::CurrentSlot;
use pallet_session::historical as pallet_session_historical;
use pallet_staking::FixedNominationsQuota;
use parity_scale_codec::Encode;
use sp_consensus_babe::{AuthorityId, AuthorityPair};
use sp_core::{
    bounded_vec,
    crypto::{KeyTypeId, Pair},
    Get, H256, U256,
};
use sp_runtime::{
    curve::PiecewiseLinear,
    impl_opaque_keys,
    testing::{Digest, DigestItem, TestXt},
    traits::{IdentityLookup, OpaqueKeys},
    BuildStorage, Perbill,
};
use supported_chains_primitives::MATURITY_FIXED_DELAY_10;

use attestor_primitives::{AttestationChainConfiguration, ChainEncodingVersion};

use sp_staking::{EraIndex, SessionIndex};

type DummyValidatorId = u64;
pub type AccountId = u64;
type Block = frame_system::mocking::MockBlock<Test>;
type Balance = u128;

pub const ALICE: AccountId = 1;

frame_support::construct_runtime!(
    pub enum Test
    {
        System: frame_system,
        Authorship: pallet_authorship,
        Balances: pallet_balances,
        Historical: pallet_session_historical,
        Offences: pallet_offences,
        Babe: pallet_babe,
        Staking: pallet_staking,
        Session: pallet_session,
        Timestamp: pallet_timestamp,
        SupportedChains: pallet_supported_chains,
        Attestation: attestation_poc,
        RandomnessPallet: pallet_randomness,
        Operators: pallet_membership::<Instance1>,
    }
);

impl frame_system::Config for Test {
    type PreInherents = ();
    type PostInherents = ();
    type PostTransactions = ();
    type RuntimeTask = RuntimeTask;
    type MultiBlockMigrator = ();
    type SingleBlockMigrations = ();
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type Nonce = u64;
    type RuntimeCall = RuntimeCall;
    type Hash = H256;
    type Version = ();
    type Hashing = sp_runtime::traits::BlakeTwo256;
    type AccountId = AccountId;
    type Lookup = IdentityLookup<Self::AccountId>;
    type Block = Block;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = ConstU64<250>;
    type PalletInfo = PalletInfo;
    type AccountData = pallet_balances::AccountData<u128>;
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type SS58Prefix = ();
    type OnSetCode = ();
    type MaxConsumers = frame_support::traits::ConstU32<16>;
    type ExtensionsWeightInfo = ();
}

impl<C> frame_system::offchain::CreateTransactionBase<C> for Test
where
    RuntimeCall: From<C>,
{
    type RuntimeCall = RuntimeCall;
    type Extrinsic = TestXt<RuntimeCall, ()>;
}

impl<C> frame_system::offchain::CreateBare<C> for Test
where
    RuntimeCall: From<C>,
{
    fn create_bare(call: Self::RuntimeCall) -> Self::Extrinsic {
        TestXt::new_bare(call)
    }
}

impl_opaque_keys! {
    pub struct MockSessionKeys {
        pub babe_authority: pallet_babe::Pallet<Test>,
    }
}

impl pallet_session::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type ValidatorId = <Self as frame_system::Config>::AccountId;
    type ValidatorIdOf = sp_runtime::traits::ConvertInto;
    type ShouldEndSession = Babe;
    type NextSessionRotation = Babe;
    type SessionManager = pallet_session::historical::NoteHistoricalRoot<Self, Staking>;
    type SessionHandler = <MockSessionKeys as OpaqueKeys>::KeyTypeIdProviders;
    type Keys = MockSessionKeys;
    type WeightInfo = ();
    type DisablingStrategy =
        pallet_session::disabling::UpToLimitDisablingStrategy<SLASHING_DISABLING_FACTOR>;
    type Currency = Balances;
    type KeyDeposit = frame_support::traits::ConstU128<0>;
}

impl pallet_session::historical::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type FullIdentification = pallet_staking::Exposure<u64, u128>;
    type FullIdentificationOf = pallet_staking::DefaultExposureOf<Self>;
}

impl pallet_authorship::Config for Test {
    type FindAuthor = pallet_session::FindAccountFromAuthorIndex<Self, Babe>;
    type EventHandler = ();
}

impl pallet_timestamp::Config for Test {
    type Moment = u64;
    type OnTimestampSet = Babe;
    type MinimumPeriod = ConstU64<1>;
    type WeightInfo = ();
}

impl pallet_balances::Config for Test {
    type MaxLocks = MaxLocks;
    type MaxReserves = ();
    type ReserveIdentifier = [u8; 8];
    type Balance = u128;
    type DustRemoval = ();
    type RuntimeEvent = RuntimeEvent;
    type ExistentialDeposit = ExistentialDeposit;
    type AccountStore = System;
    type WeightInfo = ();
    type FreezeIdentifier = ();
    type MaxFreezes = ();
    type RuntimeHoldReason = RuntimeHoldReason;
    type RuntimeFreezeReason = RuntimeFreezeReason;
    type DoneSlashHandler = ();
}

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
    pub const BondingDuration: EraIndex = 3;
    pub const SlashDeferDuration: EraIndex = 0;
    pub const RewardCurve: &'static PiecewiseLinear<'static> = &REWARD_CURVE;
    pub const OffendingValidatorsThreshold: Perbill = Perbill::from_percent(16);
    pub static ElectionsBounds: ElectionBounds = ElectionBoundsBuilder::default().build();
}

pub struct OnChainSeqPhragmen;
impl onchain::Config for OnChainSeqPhragmen {
    type System = Test;
    type Solver = SequentialPhragmen<DummyValidatorId, Perbill>;
    type DataProvider = Staking;
    type WeightInfo = ();
    type Bounds = ElectionsBounds;
    type Sort = frame_support::traits::ConstBool<false>;
    type MaxBackersPerWinner = ConstU32<256>;
    type MaxWinnersPerPage = ConstU32<100>;
}

pub const SLASHING_DISABLING_FACTOR: usize = 3;

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
    type OldCurrency = Balances;
    type RuntimeHoldReason = RuntimeHoldReason;
    type MaxValidatorSet = ConstU32<100>;
    type Filter = frame_support::traits::Nothing;
}

impl pallet_offences::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type IdentificationTuple = pallet_session::historical::IdentificationTuple<Self>;
    type OnOffenceHandler = Staking;
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
    type DisabledValidators = Session;
    type WeightInfo = ();
    type MaxAuthorities = ConstU32<10>;
    type MaxNominators = ConstU32<100>;
    type KeyOwnerProof = <Historical as KeyOwnerProofSystem<(KeyTypeId, AuthorityId)>>::Proof;
    type EquivocationReportSystem = ();
}

use attestor_primitives::BlsPublicKeyWrapper;

parameter_types! {
    pub const ExistentialDeposit: u128 = 500;
    pub const MaxLocks: u32 = 50;
    pub const MaxAttestorsDefault: u32 = 100;
    pub const CommittmentInterval: u64 = 1000;
    pub const DefaultAttestationsPerCheckpoint: u32 = 10;
    pub const DefaultAttestationInterval: u64 = 10;
    pub const DefaultTargetSampleSize: u32 = 1;
    pub const DefaultMaxCatchup: u32 = 500;
    pub const DefaultMinBondRequirement: u128 = 100_000_000_000_000_000_000; // 100 units
    pub const MaxUnlockingChunks: u32 = 10;
    pub const MaxAttestationsPerBlock: u32 = 10;
    pub const DefaultAttestationRetentionDuration: u32 = 10;
    pub const MaxCheckpointsImportedPerCall: u32 = 100;
    pub const DefaultAttestationChainGenesisBlockNumber: u64 = 0;
}

// Ensure origin for members of the Operators membership.
type EnsureOperators = frame_system::EnsureSignedBy<Operators, AccountId>;
// Ensure origin for either root or members of the Operators membership.
type EnsureRootOrOperators =
    frame_support::traits::EitherOfDiverse<frame_system::EnsureRoot<AccountId>, EnsureOperators>;

impl attestation_poc::Config for Test {
    type DefaultAttestationsPerCheckpoint = DefaultAttestationsPerCheckpoint;
    type DefaultAttestationInterval = DefaultAttestationInterval;
    type DefaultTargetSampleSize = DefaultTargetSampleSize;
    type DefaultMaxCatchup = DefaultMaxCatchup;
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = attestation_poc::weights::WeightInfo<Test>;
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
    type DefaultAttestationRetentionDuration = DefaultAttestationRetentionDuration;
    type MaxCheckpointsImportedPerCall = MaxCheckpointsImportedPerCall;
    type DefaultAttestationChainGenesisBlockNumber = DefaultAttestationChainGenesisBlockNumber;
    type OperatorsOrigin = EnsureRootOrOperators;
}

parameter_types! {
    pub const DefaultMaturityStrategy: &'static str = MATURITY_FIXED_DELAY_10;
}

impl pallet_supported_chains::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_supported_chains::weights::WeightInfo<Test>;
    type EventListeners = Attestation;
    type ChainRegistrationHandler = Attestation;
    type DefaultMaturityStrategy = DefaultMaturityStrategy;
    type OperatorsOrigin = frame_system::EnsureRoot<AccountId>;
}

impl pallet_randomness::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_randomness::weights::WeightInfo<Test>;
    type EventListeners = Attestation;
}

parameter_types! {
    pub const MaxOperators: u32 = 5;
}

// Operators membership instance. Only the sudo account can add/remove members, and there can be at most 5 members.
// This membership is used to control certain operations in the Attestation and SupportedChains pallets.
type OperatorsInstance = pallet_membership::Instance1;
impl pallet_membership::Config<OperatorsInstance> for Test {
    type RuntimeEvent = RuntimeEvent;
    type AddOrigin = frame_system::EnsureRoot<AccountId>;
    type RemoveOrigin = frame_system::EnsureRoot<AccountId>;
    type SwapOrigin = frame_system::EnsureRoot<AccountId>;
    type ResetOrigin = frame_system::EnsureRoot<AccountId>;
    type PrimeOrigin = frame_system::EnsureNever<AccountId>;
    type MembershipInitialized = ();
    type MembershipChanged = ();
    type MaxMembers = MaxOperators;
    type WeightInfo = ();
}

// add more accounts when you need them
// and update balances genesis below
pub(crate) const STASH_1: AccountId = 1;
pub(crate) const STASH_2: AccountId = 2;
pub(crate) const STASH_3: AccountId = 3;

pub(crate) const ATTESTOR_1: AccountId = 4;
pub(crate) const ATTESTOR_2: AccountId = 5;
pub(crate) const ATTESTOR_3: AccountId = 6;

// Mock source chain id
pub const SOURCE_CHAIN_ID: u64 = 200;
// Corresponding chain key for the above chain id
pub const SUPPORTED_CHAIN_KEY: u64 = 1;

#[derive(Default)]
pub struct ExtBuilder;

impl ExtBuilder {
    pub fn build(self) -> sp_io::TestExternalities {
        // Init env logger to see logs in debug mode
        let _ = env_logger::try_init();

        let mut t = frame_system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();
        // accounts 0 to 5 have initial balances
        let b = pallet_balances::GenesisConfig::<Test> {
            dev_accounts: None,
            balances: vec![
                (0, 900_000_000_000_000_000_000),
                (STASH_1, 900_000_000_000_000_000_000),
                (ATTESTOR_1, 900_000_000_000_000_000_000),
                (STASH_2, 5_000_000_000_000_000_000_000),
                (ATTESTOR_2, 5_000_000_000_000_000_000_000),
                (STASH_3, 1_000_000_000_000_000_000_000),
            ],
        };
        b.assimilate_storage(&mut t).unwrap();

        let chains = pallet_supported_chains::GenesisConfig::<Test> {
            supported_chains: vec![(
                SOURCE_CHAIN_ID,
                "Ethereum".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            )],
            _phantom: Default::default(),
        };
        chains.assimilate_storage(&mut t).unwrap();

        let att = Attestor::new(ATTESTOR_3, STASH_3);
        let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
            invulnerables: vec![(ATTESTOR_3, BlsPublicKeyWrapper(att.public_key))],
            attestation_chain_configurations: vec![AttestationChainConfiguration {
                chain_key: SUPPORTED_CHAIN_KEY,
                attestation_interval: 10,
                attestations_per_checkpoint: 10,
                target_sample_size: TargetSampleSizeDefault::<Test>::get(),
                checkpoints: vec![],
            }],
        };
        pallet_genesis.assimilate_storage(&mut t).unwrap();

        let pairs = (0..1)
            .map(|i| {
                let seed = U256::from(i).to_big_endian();
                AuthorityPair::from_seed(&seed)
            })
            .collect::<Vec<_>>();

        let public: Vec<_> = pairs.iter().map(|p| p.public()).collect();

        // stashes are the index.
        let session_keys: Vec<_> = public
            .iter()
            .enumerate()
            .map(|(i, k)| {
                (
                    i as u64,
                    i as u64,
                    MockSessionKeys {
                        babe_authority: AuthorityId::from(k.clone()),
                    },
                )
            })
            .collect();

        // NOTE: this will initialize the babe authorities
        // through OneSessionHandler::on_genesis_session
        pallet_session::GenesisConfig::<Test> {
            keys: session_keys,
            non_authority_keys: vec![],
        }
        .assimilate_storage(&mut t)
        .unwrap();

        // controllers are same as stash
        let stakers: Vec<_> = (0..public.len())
            .map(|i| {
                (
                    i as u64,
                    i as u64,
                    10_000,
                    pallet_staking::StakerStatus::<u64>::Validator,
                )
            })
            .collect();

        let staking_config = pallet_staking::GenesisConfig::<Test> {
            stakers,
            validator_count: 8,
            force_era: pallet_staking::Forcing::ForceNew,
            minimum_validator_count: 0,
            invulnerables: vec![],
            ..Default::default()
        };

        staking_config.assimilate_storage(&mut t).unwrap();

        let membership_config = pallet_membership::GenesisConfig::<Test, OperatorsInstance> {
            members: bounded_vec![ALICE],
            ..Default::default()
        };

        membership_config.assimilate_storage(&mut t).unwrap();

        t.into()
    }

    pub fn build_and_execute<R>(self, test: impl FnOnce() -> R) -> R {
        self.build().execute_with(|| {
            System::set_block_number(1);
            Timestamp::set_timestamp(1);
            test()
        })
    }
}

pub fn go_to_block(n: u64, s: u64) {
    use frame_support::traits::OnFinalize;

    // `System::initialize` (frame_system stable2512+) requires `n == block_number() + 1`.
    // `build_and_execute` sets block number to 1 without going through `initialize(1)`, so the
    // first `go_to_block(1, …)` must rewind one step before simulating that block.
    if System::block_number() == n {
        System::set_block_number(n.saturating_sub(1));
    }

    Babe::on_finalize(System::block_number());
    Session::on_finalize(System::block_number());
    Staking::on_finalize(System::block_number());
    RandomnessPallet::on_finalize(System::block_number());

    let parent_hash = if System::block_number() > 1 {
        let hdr = System::finalize();
        hdr.hash()
    } else {
        System::parent_hash()
    };

    let pre_digest = make_secondary_plain_pre_digest(0, s.into());

    System::reset_events();
    System::initialize(&n, &parent_hash, &pre_digest);

    // Set timestamp based on slot
    Timestamp::set_timestamp(*CurrentSlot::<Test>::get() * Babe::slot_duration());

    Babe::on_initialize(n);
    Session::on_initialize(n);
    Staking::on_initialize(n);
    RandomnessPallet::on_initialize(n);
    Attestation::on_initialize(n);
}

/// Slots will grow accordingly to blocks
pub fn progress_to_block(n: u64) {
    let mut slot = u64::from(Babe::current_slot()) + 1;
    for i in System::block_number() + 1..=n {
        go_to_block(i, slot);
        slot += 1;
    }
}

pub fn make_secondary_plain_pre_digest(
    authority_index: sp_consensus_babe::AuthorityIndex,
    slot: sp_consensus_babe::Slot,
) -> Digest {
    let digest_data = sp_consensus_babe::digests::PreDigest::SecondaryPlain(
        sp_consensus_babe::digests::SecondaryPlainPreDigest {
            authority_index,
            slot,
        },
    );
    let log = DigestItem::PreRuntime(sp_consensus_babe::BABE_ENGINE_ID, digest_data.encode());
    Digest { logs: vec![log] }
}
