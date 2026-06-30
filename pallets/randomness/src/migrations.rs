//! Storage migrations for pallet-randomness.

use frame_support::{
    pallet_prelude::*,
    traits::{Get, GetStorageVersion, OnRuntimeUpgrade},
    weights::Weight,
    BoundedBTreeMap,
};
use sp_std::{marker::PhantomData, vec::Vec};

#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;

use crate::pallet::{Config, Pallet, RandomnessByEpochIndex};
use randomness_primitives::Randomness;

/// The legacy storage layout of the randomness pallet (storage version 0).
mod old {
    use super::*;
    use frame_support::{storage_alias, Blake2_128Concat};

    /// `RandomnessByEpochIndex` used to be an unbounded `StorageMap`. It lives under
    /// the same pallet/storage prefix as the new `StorageValue`, but its entries carry
    /// an extra hashed-key suffix, so the two never collide on a concrete key.
    #[storage_alias]
    pub type RandomnessByEpochIndex<T: Config> =
        StorageMap<Pallet<T>, Blake2_128Concat, u64, Randomness, ValueQuery>;
}

/// Migration V0 -> V1: move the unbounded `StorageMap` of epoch randomness into a
/// `BoundedBTreeMap` `StorageValue`, keeping only the latest
/// [`Config::MaxRandomnessEntries`] epochs (those with the highest epoch index).
pub struct MigrateRandomnessByEpochIndexV0ToV1<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateRandomnessByEpochIndexV0ToV1<T> {
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!(
                target: "runtime::randomness",
                "RandomnessByEpochIndex migration: already at {target:?} or above (on_chain={on_chain:?}), skipping"
            );
            return T::DbWeight::get().reads(1);
        }

        let max = T::MaxRandomnessEntries::get() as usize;

        // Drain every legacy entry, removing it from storage as we go. Draining first
        // (before writing the new `StorageValue`) avoids any chance of the new value's
        // key being swept up by the map's prefix.
        let mut entries: Vec<(u64, Randomness)> =
            old::RandomnessByEpochIndex::<T>::drain().collect();
        let drained = entries.len() as u64;

        // Keep only the latest `max` epochs (highest epoch indices).
        entries.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        entries.truncate(max);

        let mut map: BoundedBTreeMap<u64, Randomness, T::MaxRandomnessEntries> =
            BoundedBTreeMap::new();
        for (epoch_index, randomness) in entries {
            // Safe: we truncated to `max`, which is the bound of the map.
            if map.try_insert(epoch_index, randomness).is_err() {
                log::error!(
                    target: "runtime::randomness",
                    "RandomnessByEpochIndex migration: unexpected overflow inserting epoch {epoch_index}"
                );
                break;
            }
        }
        let kept = map.len() as u64;

        RandomnessByEpochIndex::<T>::put(map);
        target.put::<Pallet<T>>();

        log::info!(
            target: "runtime::randomness",
            "RandomnessByEpochIndex migration: drained {drained} legacy entries, kept latest {kept}"
        );

        // reads: on-chain version + every drained entry.
        // writes: every drained removal + the new value + the new version.
        T::DbWeight::get().reads_writes(drained + 1, drained + 2)
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        use parity_scale_codec::Encode;

        let on_chain = Pallet::<T>::on_chain_storage_version();
        if on_chain >= StorageVersion::new(1) {
            log::info!(
                target: "runtime::randomness",
                "pre_upgrade: already migrated (on_chain={on_chain:?}), skipping"
            );
            return Ok((0u64, false).encode());
        }

        let count = old::RandomnessByEpochIndex::<T>::iter().count() as u64;
        let expected_kept = count.min(T::MaxRandomnessEntries::get() as u64);

        log::info!(
            target: "runtime::randomness",
            "pre_upgrade: found {count} legacy entries, expect to keep {expected_kept}"
        );

        Ok((expected_kept, true).encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
        use parity_scale_codec::Decode;

        let (expected_kept, should_run): (u64, bool) =
            Decode::decode(&mut &state[..]).map_err(|_| "failed to decode pre_upgrade state")?;

        if !should_run {
            log::info!(target: "runtime::randomness", "post_upgrade: migration was skipped");
            return Ok(());
        }

        let on_chain = Pallet::<T>::on_chain_storage_version();
        let current = Pallet::<T>::in_code_storage_version();
        ensure!(
            on_chain == current,
            "post_upgrade: storage version not updated"
        );

        let kept = RandomnessByEpochIndex::<T>::get().len() as u64;
        ensure!(
            kept == expected_kept,
            "post_upgrade: retained entry count mismatch"
        );

        ensure!(
            old::RandomnessByEpochIndex::<T>::iter().next().is_none(),
            "post_upgrade: legacy entries still present"
        );

        log::info!(
            target: "runtime::randomness",
            "post_upgrade: verified {kept} retained randomness entries"
        );
        Ok(())
    }
}
