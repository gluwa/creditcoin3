use crate::RUNTIME_API_VERSIONS;
use sp_version::RuntimeVersion;

#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: std::borrow::Cow::Borrowed("creditcoin3"),
    impl_name: std::borrow::Cow::Borrowed("creditcoin3"),
    authoring_version: 3,
    spec_version: 106,
    impl_version: 0,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 2,
    system_version: 1,
};
