use sp_std::vec::Vec;

use attestor_primitives::ChainId;

sp_api::decl_runtime_apis! {
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_id: ChainId) -> bool;

        fn supported_chains() -> Option<Vec<ChainId>>;
    }
}
