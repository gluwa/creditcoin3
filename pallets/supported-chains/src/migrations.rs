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

use crate::pallet::{Config, CoreFees, Pallet};

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

/// Migration V1 -> V2: drops the `token` field from `CoreFeeConfig`.
///
/// The core fee is always denominated in attestcoin — the Outbox pulls it with `transferFrom` and
/// has no native-currency path — so a configurable token could only ever disagree with what is
/// actually charged. Removing it also matches the `get_core_fee(uint32) -> uint256` ABI the
/// contracts consume.
///
/// `token` was the *first* field of the struct, so any pre-existing entry would now mis-decode
/// (a leading `Option<H160>` read as the start of a `U256`). `CoreFees` has no genesis field and
/// write-ability is not deployed on any network, so the map is expected to be empty everywhere and
/// this should remove nothing. It clears the map regardless rather than trusting that expectation:
/// a stale entry that silently decoded to a wrong fee would mischarge every publish on that chain.
pub struct MigrateV1ToV2<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateV1ToV2<T> {
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(2);

        if on_chain >= target {
            log::info!(
                target: "runtime::supported_chains",
                "CoreFeeConfig migration: already at {target:?} or above (on_chain={on_chain:?}), skipping"
            );
            return T::DbWeight::get().reads(1);
        }

        // `None` cursor: start from the beginning. The limit is a safety bound, not an expectation —
        // one entry per supported chain is the theoretical maximum and the map should be empty.
        let cleared = CoreFees::<T>::clear(u32::MAX, None);

        target.put::<Pallet<T>>();

        log::info!(
            target: "runtime::supported_chains",
            "CoreFeeConfig migration: cleared {} legacy core-fee entr(ies) and bumped storage \
             version {on_chain:?} -> {target:?}. Re-set fees with set_core_fee (amount only).",
            cleared.unique,
        );

        // reads/writes: the on-chain version plus one read+write per cleared entry, and the version write.
        T::DbWeight::get().reads_writes(
            1u64.saturating_add(cleared.loops.into()),
            1u64.saturating_add(cleared.unique.into()),
        )
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        Ok(Vec::new())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(_state: Vec<u8>) -> Result<(), TryRuntimeError> {
        ensure!(
            Pallet::<T>::on_chain_storage_version() >= StorageVersion::new(2),
            "post_upgrade: storage version not updated"
        );
        ensure!(
            CoreFees::<T>::iter().next().is_none(),
            "post_upgrade: CoreFees still has entries after the clear"
        );
        Ok(())
    }
}
