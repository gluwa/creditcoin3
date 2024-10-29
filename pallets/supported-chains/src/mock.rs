use crate as supported_chains;
use crate::ChainId;
use frame_support::traits::{ConstU16, ConstU64};
use sp_core::H256;
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};

pub type AccountId = u64;
type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
    pub enum Test
    {
        System: frame_system,
        SupportedChain: supported_chains,
    }
);

impl frame_system::Config for Test {
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Nonce = u64;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = AccountId;
    type Lookup = IdentityLookup<Self::AccountId>;
    type Block = Block;
    type RuntimeEvent = RuntimeEvent;
    type BlockHashCount = ConstU64<250>;
    type Version = ();
    type PalletInfo = PalletInfo;
    type AccountData = ();
    type OnNewAccount = ();
    type OnKilledAccount = ();
    type SystemWeightInfo = ();
    type SS58Prefix = ConstU16<42>;
    type OnSetCode = ();
    type MaxConsumers = frame_support::traits::ConstU32<16>;
}

impl supported_chains::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = supported_chains::weights::WeightInfo<Test>;
}

#[derive(Default)]
pub struct ExtBuilder;

impl ExtBuilder {
    pub fn with_supported_chains(self) -> sp_io::TestExternalities {
        let mut storage = frame_system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();

        let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
            supported_chains: vec![(200, "Ethereum".as_bytes().to_vec())],
            _phantom: Default::default(),
        };

        pallet_genesis.assimilate_storage(&mut storage).unwrap();

        storage.into()
    }

    pub fn build_and_execute(self, test: impl FnOnce()) {
        self.with_supported_chains().execute_with(test);
    }

    pub fn build_and_execute_with_duplicate_chains(
        self,
        supported_chains: Vec<(ChainId, Vec<u8>)>,
        test: impl FnOnce(),
    ) {
        let mut storage = frame_system::GenesisConfig::<Test>::default()
            .build_storage()
            .unwrap();

        let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
            supported_chains,
            _phantom: Default::default(),
        };

        pallet_genesis.assimilate_storage(&mut storage).unwrap();

        let mut ext: sp_io::TestExternalities = storage.into();
        ext.execute_with(test);
    }
}

pub fn new_test_ext() -> sp_io::TestExternalities {
    frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap()
        .into()
}
