use sp_std::vec::Vec;

use attestor_primitives::{ChainId, ChainKey};
use sp_core::H160;

use crate::WriteAbilityConfig;

sp_api::decl_runtime_apis! {
    // v2 added the write-ability methods (`write_ability_config`, `outbox_factory_address`) to the
    // v1 API, letting rolling clients distinguish a pre-write-ability (v1) runtime from a v2 one
    // via `RuntimeApi::api_version` instead of silently failing the call (audit P2-9).
    //
    // v3 adds `outbox_discovery_address`, mirroring `outbox_factory_address`, so callers off the
    // chain-info precompile can resolve the Outbox from a registry read instead of scanning the
    // permissionless factory's `OutboxCreated` logs. `sp_api::decl_runtime_apis!` requires every
    // per-method `#[api_version(K)]` to be >= the trait's own declared version, so the old v2 tags
    // on `write_ability_config`/`outbox_factory_address` had to drop once the trait moved to v3 —
    // this crate only ever implements the single current API (no versioned client-side aliasing),
    // so that loses no behaviour: a caller's `api_version() >= 2` check still passes once the
    // runtime reports v3.
    #[api_version(3)]
    pub trait SupportedChainsApi
    {
        fn is_chain_supported(chain_key: ChainKey) -> bool;

        fn supported_chains() -> Vec<ChainKey>;

        fn chain_key_by_chain_id_and_name(chain_id: ChainId, chain_name: Vec<u8>) -> Option<ChainKey>;

        fn write_ability_config(chain_key: ChainKey) -> Option<WriteAbilityConfig>;

        fn outbox_factory_address(chain_key: ChainKey) -> Option<H160>;

        fn outbox_discovery_address(chain_key: ChainKey) -> Option<H160>;
    }
}
