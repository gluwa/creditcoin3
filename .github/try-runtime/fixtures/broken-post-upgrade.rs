//! try-runtime self-test fixture: a single-block migration whose `post_upgrade` fails.
//!
//! Never part of the real runtime. `.github/try-runtime/fixtures/apply.sh` injects it
//! into `SingleBlockMigrations` for `.github/workflows/try-runtime-selftest.yml`, which
//! expects the try-runtime check to fail with the message below.
//!
//! It is a `VersionedMigration`, the shape a single-pass check misses (see
//! `.github/try-runtime/run.sh`).
//!
//! The version it steps belongs to `SelftestPallet`, a stand-in rather than a real
//! pallet. Every real pallet's on-chain version already equals its in-code version, so a
//! migration on one would either not apply or trip the pallet's own version check, and
//! fail for the wrong reason. Nothing on chain has written the stand-in's version, so it
//! reads as 0 and the migration applies, just as a pending one would.

use frame_support::{
    migrations::VersionedMigration,
    traits::{
        CrateVersion, GetStorageVersion, PalletInfoAccess, StorageVersion,
        UncheckedOnRuntimeUpgrade,
    },
    weights::Weight,
};

use crate::Runtime;

pub type Fixture = VersionedMigration<
    0,
    1,
    DeliberatelyBrokenPostUpgrade,
    SelftestPallet,
    <Runtime as frame_system::Config>::DbWeight,
>;

pub struct SelftestPallet;

impl PalletInfoAccess for SelftestPallet {
    fn index() -> usize {
        // Not a pallet index in construct_runtime!.
        255
    }
    fn name() -> &'static str {
        "TryRuntimeSelftest"
    }
    fn name_hash() -> [u8; 16] {
        sp_core::hashing::twox_128(Self::name().as_bytes())
    }
    fn module_name() -> &'static str {
        "try_runtime_selftest"
    }
    fn crate_version() -> CrateVersion {
        CrateVersion::new(0, 0, 0)
    }
}

impl GetStorageVersion for SelftestPallet {
    type InCodeStorageVersion = StorageVersion;

    fn in_code_storage_version() -> Self::InCodeStorageVersion {
        StorageVersion::new(1)
    }
    fn on_chain_storage_version() -> StorageVersion {
        StorageVersion::get::<Self>()
    }
}

pub struct DeliberatelyBrokenPostUpgrade;

impl UncheckedOnRuntimeUpgrade for DeliberatelyBrokenPostUpgrade {
    fn on_runtime_upgrade() -> Weight {
        log::info!("try-runtime selftest: broken-post-upgrade fixture applied");
        Weight::zero()
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(_state: alloc::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
        Err("try-runtime selftest: deliberate post_upgrade failure".into())
    }
}
