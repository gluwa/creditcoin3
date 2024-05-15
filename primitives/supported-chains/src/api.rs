use sp_std::prelude::*;

sp_api::decl_runtime_apis! {
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_id: u64) -> bool;

        fn supported_chains() -> Option<Vec<u64>>;
    }
}
