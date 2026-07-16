use sp_std::vec::Vec;

use attestor_primitives::{ChainId, ChainKey};
use sp_core::H160;

use crate::WriteAbilityConfig;

sp_api::decl_runtime_apis! {
    // v2 adds the write-ability methods (`write_ability_config`, `outbox_factory_address`) to the
    // v1 API. Versioning them explicitly lets rolling clients distinguish a pre-write-ability (v1)
    // runtime from a v2 one via `RuntimeApi::api_version`, instead of silently failing the call
    // (audit P2-9).
    #[api_version(2)]
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_key: ChainKey) -> bool;

        fn supported_chains() -> Vec<ChainKey>;

        fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;

        #[api_version(2)]
        fn write_ability_config(chain_key: ChainKey) -> Option<WriteAbilityConfig>;

        #[api_version(2)]
        fn outbox_factory_address(chain_key: ChainKey) -> Option<H160>;
    }
}
