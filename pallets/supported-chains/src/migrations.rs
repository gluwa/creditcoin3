//! Storage migrations for pallet-supported-chains.

use frame_support::{
    pallet_prelude::*,
    traits::{Get, GetStorageVersion, OnRuntimeUpgrade},
    weights::Weight,
};
use sp_std::marker::PhantomData;

#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
#[cfg(feature = "try-runtime")]
use sp_std::vec::Vec;

use crate::pallet::{Config, Pallet};

/// Migration V0 -> V1: introduces the `WriteAbilityConfigs` companion storage map.
///
/// The map is a brand-new `OptionQuery` `StorageMap`, so there is no legacy data to transform —
/// absent entries simply decode to `None`. This migration only bumps the on-chain
/// `StorageVersion` so it stays in sync with the in-code version for future migrations.
pub struct MigrateV0ToV1<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateV0ToV1<T> {
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!(
                target: "runtime::supported_chains",
                "WriteAbilityConfigs migration: already at {target:?} or above (on_chain={on_chain:?}), skipping"
            );
            return T::DbWeight::get().reads(1);
        }

        target.put::<Pallet<T>>();

        log::info!(
            target: "runtime::supported_chains",
            "WriteAbilityConfigs migration: bumped storage version {on_chain:?} -> {target:?}"
        );

        // reads: on-chain version. writes: the new version.
        T::DbWeight::get().reads_writes(1, 1)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        Ok(Vec::new())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(_state: Vec<u8>) -> Result<(), TryRuntimeError> {
        ensure!(
            Pallet::<T>::on_chain_storage_version() >= StorageVersion::new(1),
            "post_upgrade: storage version not updated"
        );
        Ok(())
    }
}
