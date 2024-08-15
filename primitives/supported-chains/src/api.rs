use sp_std::vec::Vec;

use attestor_primitives::ChainId;

sp_api::decl_runtime_apis! {
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_id: ChainId) -> bool;

        fn supported_chains() -> Option<Vec<ChainId>>;

        fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainId>;
    }
}
