//! Storage migrations for pallet-supported-chains.

use frame_support::{
    pallet_prelude::*,
    traits::{Get, GetStorageVersion, OnRuntimeUpgrade},
    weights::Weight,
};
use sp_std::marker::PhantomData;

#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
use sp_std::vec::Vec;

use crate::{
    is_reserved_for_another_chain,
    pallet::{Config, CoreFees, Pallet, WriteAbilityChainKeyOwners, WriteAbilityConfigs},
};

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

    /// Records whether the migration will run (on-chain version below 2). `post_upgrade` may only
    /// assert the map is empty in that case: on a chain already at V2 the migration is skipped and
    /// `CoreFees` legitimately holds the fees set since (usc-devnet had two when it went 131 → 136).
    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        use parity_scale_codec::Encode;
        let will_run = Pallet::<T>::on_chain_storage_version() < StorageVersion::new(2);
        Ok(will_run.encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
        use parity_scale_codec::Decode;
        ensure!(
            Pallet::<T>::on_chain_storage_version() >= StorageVersion::new(2),
            "post_upgrade: storage version not updated"
        );
        let ran = bool::decode(&mut &state[..])
            .map_err(|_| TryRuntimeError::Other("post_upgrade: bad pre_upgrade state"))?;
        if ran {
            ensure!(
                CoreFees::<T>::iter().next().is_none(),
                "post_upgrade: CoreFees still has entries after the clear"
            );
        }
        Ok(())
    }
}

/// Builds [`WriteAbilityChainKeyOwners`] from the `WriteAbilityConfigs` that exist before it does.
///
/// Not gated on the pallet's storage version (the index is a plain new map, like
/// `InitSupportedChainsCount`), and idempotent, so it is harmless to leave in the runtime until
/// every network has run it. Keys are meant to be unique, so a conflict (two chains on one key, or
/// a key in another chain's default range) is only logged: the lowest chain key is bound, the
/// config is left alone and the upgrade is not aborted. `post_upgrade` fails on one, so a
/// try-runtime rehearsal catches it. None exists on a live network (asc-devnet checked 2026-11-24).
pub struct IndexWriteAbilityChainKeys<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for IndexWriteAbilityChainKeys<T> {
    fn on_runtime_upgrade() -> Weight {
        let mut configs: Vec<_> = WriteAbilityConfigs::<T>::iter()
            .map(|(chain_key, config)| (chain_key, config.write_ability_chain_key))
            .collect();
        // `iter` order follows the hasher, so sort for a deterministic "lowest chain key wins".
        configs.sort_unstable_by_key(|(chain_key, _)| *chain_key);

        let mut writes = 0u64;
        for (chain_key, key) in &configs {
            if is_reserved_for_another_chain(key, *chain_key) {
                log::error!(
                    target: "runtime::supported_chains",
                    "chain {chain_key} uses the default write_ability_chain_key of another chain"
                );
            }
            match WriteAbilityChainKeyOwners::<T>::get(key) {
                None => {
                    WriteAbilityChainKeyOwners::<T>::insert(key, chain_key);
                    writes = writes.saturating_add(1);
                }
                Some(owner) if owner == *chain_key => {}
                Some(owner) => log::error!(
                    target: "runtime::supported_chains",
                    "chain {chain_key} shares its write_ability_chain_key with chain {owner}"
                ),
            }
        }

        let reads = (configs.len() as u64).saturating_mul(2);
        T::DbWeight::get().reads_writes(reads, writes)
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(_state: Vec<u8>) -> Result<(), TryRuntimeError> {
        for (chain_key, config) in WriteAbilityConfigs::<T>::iter() {
            let key = config.write_ability_chain_key;
            ensure!(
                WriteAbilityChainKeyOwners::<T>::get(key) == Some(chain_key),
                "post_upgrade: a write_ability_chain_key is shared or not bound to its chain"
            );
            ensure!(
                !is_reserved_for_another_chain(&key, chain_key),
                "post_upgrade: a chain uses the default write_ability_chain_key of another chain"
            );
        }
        Ok(())
    }
}
