//! Pallet Supported Chains Benchmarks
use super::Pallet as SupportedChains;
use super::*;
use attestor_primitives::{ChainEncodingVersion, ChainId};
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use frame_support::traits::OriginTrait;
use scale_info::prelude::string::String;
use supported_chains_primitives::MATURITY_EVM_FINALIZED;

#[benchmarks]
mod benchmarks {
    use super::*;

    #[benchmark]
    fn register_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;
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
            None,
            chain_encoding,
        )
    }

    #[benchmark]
    fn remove_chain() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;
        let chain_name: String = String::from("Ethereum");
        let chain_encoding = ChainEncodingVersion::V1;

        assert_ok!(SupportedChains::<T>::register_chain(
            root_origin.clone(),
            chain_id,
            chain_name,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            chain_encoding,
        ));

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            true,
        )
    }

    #[benchmark]
    fn set_maturity_strategy() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;
        let chain_name: String = String::from("Ethereum");
        let chain_encoding = ChainEncodingVersion::V1;
        let maturity_strategy = String::from(MATURITY_EVM_FINALIZED);

        assert_ok!(SupportedChains::<T>::register_chain(
            root_origin.clone(),
            chain_id,
            chain_name,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            chain_encoding,
        ));

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            maturity_strategy,
        )
    }
}
