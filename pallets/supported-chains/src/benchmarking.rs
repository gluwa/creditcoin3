//! Pallet Supported Chains Benchmarks
use super::Pallet as SupportedChains;
use super::*;
use attestor_primitives::ChainId;
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use frame_support::traits::OriginTrait;
use scale_info::prelude::string::String;

#[benchmarks]
mod benchmarks {
    use super::*;

    #[benchmark]
    fn register_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;
        let chain_name: String = String::from("Ethereum");

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            chain_name,
        )
    }

    #[benchmark]
    fn remove_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;
        let chain_name: String = String::from("Ethereum");
        assert_ok!(SupportedChains::<T>::register_chain(
            root_origin.clone(),
            chain_id,
            chain_name
        ));

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            true,
        )
    }
}
