//! Storage migrations for pallet-randomness.
//!
//! ## V0 -> V1: seed the pruning queue with the historical backlog
//!
//! Under storage version 0 the pallet inserted one [`RandomnessByEpochIndex`] entry per
//! epoch and never removed any of them, so a live chain has accumulated one entry for
//! every epoch since genesis.
//!
//! The pruning logic introduced alongside this migration only schedules the *incremental*
//! gap each new epoch (`last_seen - max_history` .. `epoch - max_history`). On the first
//! block after the upgrade that gap is only `MaxEpochHistory` entries wide, so the queue
//! would never reach back and remove the entries from epoch `0` up to
//! `last_seen - max_history - 1`. They would remain in storage forever.
//!
//! This migration seeds [`PruningQueue`] so it spans the entire backlog
//! (`PruningState { next: 0, to: last_seen - max_history }`). The existing per-block
//! draining in `on_initialize` then removes one stale entry per block, avoiding a
//! single-block weight spike — the migration itself is O(1) and never iterates the map.

use frame_support::{
    pallet_prelude::*,
    traits::{GetStorageVersion, OnRuntimeUpgrade},
    weights::Weight,
};
#[cfg(feature = "try-runtime")]
use parity_scale_codec::{DecodeAll, Encode};
use sp_runtime::traits::Get;
#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
use sp_std::marker::PhantomData;
#[cfg(feature = "try-runtime")]
use sp_std::vec::Vec;

use crate::{
    pallet::{Config, LastSeenEpochIndex, Pallet, PruningQueue},
    PruningState,
};

/// Migration V0 -> V1: seed [`PruningQueue`] to cover the un-pruned historical backlog.
pub struct MigratePruningQueueV0ToV1<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigratePruningQueueV0ToV1<T> {
    fn on_runtime_upgrade() -> Weight {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!(
                target: "runtime::randomness",
                "PruningQueue migration: already at v1 or above (on_chain={on_chain:?}), skipping"
            );
            return T::DbWeight::get().reads(1);
        }

        log::info!(
            target: "runtime::randomness",
            "PruningQueue migration: upgrading from {on_chain:?} to {target:?}"
        );

        let last_seen = LastSeenEpochIndex::<T>::get();
        let max_history = T::MaxEpochHistory::get();

        // Reads: on-chain storage version + LastSeenEpochIndex.
        let mut weight = T::DbWeight::get().reads(2);

        // The highest epoch index that is stale under the new retention policy (inclusive).
        // If `last_seen <= max_history` the whole history is still within the retention
        // window, so there is nothing to prune and no queue to seed.
        if let Some(prune_to) = last_seen.checked_sub(max_history) {
            PruningQueue::<T>::mutate(|maybe_state| match maybe_state {
                // Defensive: if a queue already exists (e.g. the new `on_initialize` ran for
                // a few blocks before this migration landed), widen it to cover the backlog
                // rather than clobbering in-flight progress.
                Some(state) => {
                    state.next = 0;
                    state.to = state.to.max(prune_to);
                }
                None => {
                    *maybe_state = Some(PruningState {
                        next: 0,
                        to: prune_to,
                    });
                }
            });
            weight = weight.saturating_add(T::DbWeight::get().reads_writes(1, 1));

            log::info!(
                target: "runtime::randomness",
                "PruningQueue migration: scheduled backlog pruning for epochs 0..={prune_to}"
            );
        } else {
            log::info!(
                target: "runtime::randomness",
                "PruningQueue migration: nothing to prune (last_seen={last_seen}, max_history={max_history})"
            );
        }

        target.put::<Pallet<T>>();
        weight.saturating_add(T::DbWeight::get().writes(1))
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, TryRuntimeError> {
        let on_chain = Pallet::<T>::on_chain_storage_version();
        let target = StorageVersion::new(1);

        if on_chain >= target {
            log::info!(
                target: "runtime::randomness",
                "PruningQueue pre_upgrade: already at v1 or above (on_chain={on_chain:?}), skipping"
            );
            // (should_run, expected_to): no run, sentinel `to`.
            return Ok((false, 0u64).encode());
        }

        let last_seen = LastSeenEpochIndex::<T>::get();
        let max_history = T::MaxEpochHistory::get();

        match last_seen.checked_sub(max_history) {
            Some(prune_to) => {
                log::info!(
                    target: "runtime::randomness",
                    "PruningQueue pre_upgrade: expecting backlog pruning to epoch {prune_to}"
                );
                Ok((true, prune_to).encode())
            }
            None => {
                log::info!(
                    target: "runtime::randomness",
                    "PruningQueue pre_upgrade: no backlog to prune"
                );
                // Migration runs (bumps version) but seeds no queue.
                Ok((false, 0u64).encode())
            }
        }
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), TryRuntimeError> {
        let (expect_queue, expected_to): (bool, u64) = DecodeAll::decode_all(&mut &state[..])
            .map_err(|_| "PruningQueue post_upgrade: failed to decode pre_upgrade state")?;

        let on_chain = Pallet::<T>::on_chain_storage_version();
        let current = Pallet::<T>::in_code_storage_version();
        ensure!(
            on_chain == current,
            "PruningQueue post_upgrade: storage version not updated"
        );

        match PruningQueue::<T>::get() {
            Some(queue) => {
                ensure!(
                    expect_queue,
                    "PruningQueue post_upgrade: queue was seeded but none was expected"
                );
                ensure!(
                    queue.next == 0,
                    "PruningQueue post_upgrade: queue does not start at epoch 0"
                );
                ensure!(
                    queue.to >= expected_to,
                    "PruningQueue post_upgrade: queue does not cover the full backlog"
                );
            }
            None => {
                ensure!(
                    !expect_queue,
                    "PruningQueue post_upgrade: expected a seeded queue but found none"
                );
            }
        }

        log::info!(
            target: "runtime::randomness",
            "PruningQueue post_upgrade: checks passed"
        );
        Ok(())
    }
}
