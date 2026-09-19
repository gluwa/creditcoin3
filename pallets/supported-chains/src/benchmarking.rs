//! Pallet Supported Chains Benchmarks
use super::Pallet as SupportedChains;
use super::*;
use attestor_primitives::{ChainEncodingVersion, ChainId};
use frame_benchmarking::v2::*;
use frame_support::assert_ok;
use frame_support::traits::OriginTrait;
use scale_info::prelude::string::String;
use supported_chains_primitives::{MATURITY_EVM_FINALIZED, MATURITY_EVM_SAFE};

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
            chain_encoding,
            None,
        )
    }

    #[benchmark]
    fn set_maturity_strategy() {
        // Setup
        let root_origin = <T as frame_system::Config>::RuntimeOrigin::root();
        let chain_id: ChainId = 2;
        let chain_name: String = String::from("Ethereum");
        let chain_encoding = ChainEncodingVersion::V1;

        assert_ok!(SupportedChains::<T>::register_chain(
            root_origin.clone(),
            chain_id,
            chain_name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            chain_encoding,
            Some(String::from(MATURITY_EVM_SAFE)),
        ));

        let chain_key = ChainIdAndNameToUniqKey::<T>::get(chain_id, chain_name.as_bytes().to_vec())
            .expect("chain was just registered");

        // A strategy that differs from the registered one: an equal value is rejected with
        // `MaturityStrategyUnchanged`, so the benchmark would measure a failing call.
        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_key,
            String::from(MATURITY_EVM_FINALIZED),
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
            chain_encoding,
            None,
        ));

        #[extrinsic_call]
        _(
            root_origin as <T as frame_system::Config>::RuntimeOrigin,
            chain_id,
            true,
        )
    }
}
