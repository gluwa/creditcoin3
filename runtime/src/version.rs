use crate::RUNTIME_API_VERSIONS;
use sp_version::{create_runtime_str, RuntimeVersion};

#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: create_runtime_str!("creditcoin3"),
    impl_name: create_runtime_str!("creditcoin3"),
    authoring_version: 3,
    spec_version: 106,
    impl_version: 0,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 2,
    system_version: 1,
};
