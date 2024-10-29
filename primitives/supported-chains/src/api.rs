use sp_std::vec::Vec;

use attestor_primitives::{ChainId, ChainKey};

sp_api::decl_runtime_apis! {
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_key: ChainKey) -> bool;

        fn supported_chains() -> Option<Vec<ChainKey>>;

        fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;
    }
}
