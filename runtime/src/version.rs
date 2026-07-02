use crate::RUNTIME_API_VERSIONS;
use sp_version::{Cow, RuntimeVersion};

#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
    spec_name: Cow::Borrowed("creditcoin3"),
    impl_name: Cow::Borrowed("creditcoin3"),
    authoring_version: 3,
    spec_version: 128,
    impl_version: 0,
    apis: RUNTIME_API_VERSIONS,
    transaction_version: 3,
    system_version: 1,
};
