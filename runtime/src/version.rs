use crate::RUNTIME_API_VERSIONS;
use sp_runtime::create_runtime_str;
use sp_version::RuntimeVersion;

#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: create_runtime_str!("frontier-template"),
    impl_name: create_runtime_str!("frontier-template"),
    authoring_version: 3,
    spec_version: 4,
    impl_version: 2,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 1,
    state_version: 1,
};
