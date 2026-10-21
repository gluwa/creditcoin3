//! Storage migrations for pallet-supported-chains.

use frame_support::{
    pallet_prelude::*,
    traits::{GetStorageVersion, OnRuntimeUpgrade},
    weights::Weight,
};
use sp_std::marker::PhantomData;

#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
#[cfg(feature = "try-runtime")]
use sp_std::vec::Vec;

use crate::pallet::{Config, Pallet, SupportedChains, SupportedChainsCount};

/// Migration V0 -> V1: populate [`SupportedChainsCount`] from the existing [`SupportedChains`]
/// entries so the `MaxSupportedChains` check in `register_chain` is O(1).
pub struct MigrateSupportedChainsCountV0ToV1<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateSupportedChainsCountV0ToV1<T> {
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!(
                target: "runtime::supported-chains",
                "SupportedChainsCount migration: already at {target:?} or above (on_chain={on_chain:?}), skipping"
            );
            return T::DbWeight::get().reads(1);
        }

        let count = SupportedChains::<T>::iter_keys().count() as u32;
        SupportedChainsCount::<T>::put(count);
        target.put::<Pallet<T>>();

        log::info!(
            target: "runtime::supported-chains",
            "SupportedChainsCount migration: counted {count} chain(s)"
        );

        // reads: on-chain version + every counted chain.
        // writes: the new count + the new version.
        T::DbWeight::get().reads_writes(u64::from(count).saturating_add(1), 2)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        use parity_scale_codec::Encode;

        let on_chain = Pallet::<T>::on_chain_storage_version();
        if on_chain >= StorageVersion::new(1) {
            return Ok((0u32, false).encode());
        }

        let count = SupportedChains::<T>::iter_keys().count() as u32;
        log::info!(
            target: "runtime::supported-chains",
            "pre_upgrade: found {count} supported chain(s)"
        );
        Ok((count, true).encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
        use parity_scale_codec::Decode;

        let (expected, should_run): (u32, bool) =
            Decode::decode(&mut &state[..]).map_err(|_| "failed to decode pre_upgrade state")?;

        if !should_run {
            return Ok(());
        }

        ensure!(
            SupportedChainsCount::<T>::get() == expected,
            "SupportedChainsCount does not match the number of supported chains"
        );
        ensure!(
            Pallet::<T>::on_chain_storage_version() == StorageVersion::new(1),
            "storage version was not bumped to 1"
        );

        Ok(())
    }
}
