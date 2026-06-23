use sp_std::vec::Vec;

use attestor_primitives::{ChainId, ChainKey};
use sp_core::H160;

use crate::WriteAbilityConfig;

sp_api::decl_runtime_apis! {
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_key: ChainKey) -> bool;

        fn supported_chains() -> Vec<ChainKey>;

        fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;

        fn write_ability_config(chain_key: ChainKey) -> Option<WriteAbilityConfig>;

        fn outbox_factory_address(chain_key: ChainKey) -> Option<H160>;
    }
}
