//! Pallet Supported Chains Benchmarks
use super::*;
use attestor_primitives::{ChainEncodingVersion, ChainId, ChainKey};
use frame_benchmarking::v2::*;
use frame_support::traits::OriginTrait;
use scale_info::prelude::string::String;
use sp_core::H160;

#[benchmarks]
mod benchmarks {
    use super::*;

    #[benchmark]
    fn register_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 3099;
        let chain_name: String = String::from("Ethereum");
        let chain_encoding = ChainEncodingVersion::V1;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            chain_name,
            None,
            None,
            None,
            None,
            None,
            None,
            chain_encoding,
            None,
        )
    }

    #[benchmark]
    fn remove_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        // In mock.rs we set up a single supported chain with chain_key 1
        let chain_key: ChainKey = 1;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_key,
            true,
        )
    }

    #[benchmark]
    fn set_outbox_factory_addr() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        // In mock.rs we set up a single supported chain with chain_key 1
        let chain_key: ChainKey = 1;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_key,
            H160::zero(),
        )
    }

    #[benchmark]
    fn set_write_ability_config() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        // In mock.rs we set up a single supported chain with chain_key 1
        let chain_key: ChainKey = 1;

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_key,
            [0u8; 32],
            true,
        )
    }
}
