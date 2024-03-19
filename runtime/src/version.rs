use crate::RUNTIME_API_VERSIONS;
use sp_runtime::create_runtime_str;
use sp_version::RuntimeVersion;

#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: create_runtime_str!("creditcoin3"),
    impl_name: create_runtime_str!("creditcoin3"),
    authoring_version: 3,
    spec_version: 22,
    impl_version: 0,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 2,
    state_version: 1,
};
