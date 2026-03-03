use frame_support::{pallet_prelude::*, transactional};

use attestor_primitives::{AttestationCheckpoint, ChainKey, Digest};
use sp_std::vec::Vec;
use supported_chains_primitives::chain_removal_listener::ChainRemovalListener;

use super::pallet::*;

// This should be a reasonable value used both for clearing checkpoints
// themselves, and for clearing entries from `CheckpointBuckets`.
// Max storage writes per block = MAX_CHECKPOINTS_CLEARED_PER_BLOCK * 2;
// This happens when removing from both `Checkpoints` and `CheckpointBuckets`
pub const MAX_CHECKPOINTS_CLEARED_PER_BLOCK: u8 = 40;

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct CheckpointPruningState {
    pub stop_height: u64, // This is the height of the last checkpoint before reversion was initiated
    pub next_pivot: u64,  // inclusive lower bound for scanning pivots
}

// Benchmarking our checkpoint clearing implementation for on_initialize
// wasn't possible without this strange looking wrapper. Basically, it
// hasn't been possible so far to generate and store a checkpoint clearing
// cursor in benchmarks. So instead we allow for the storage of a cursor
// wrapper which allows the cursor to be null only during benchmarks.
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct ClearingCursor {
    pub is_benchmark: bool,
    pub inner: Option<Vec<u8>>,
}

impl ClearingCursor {
    pub fn new(cursor: Option<Vec<u8>>) -> Self {
        ClearingCursor {
            is_benchmark: false,
            inner: cursor,
        }
    }
}

impl<T: Config> Pallet<T> {
    #[transactional]
    pub(crate) fn do_revert_to(
        chain_key: ChainKey,
        checkpoint_height: u64,
    ) -> Result<Digest, sp_runtime::DispatchError> {
        let retention_duration = AttestationRetentionDuration::<T>::get(chain_key);
        let checkpoint_interval = AttestationCheckpointInterval::<T>::get(chain_key);

        // Clearing attestations. We should never have more than 2 checkpoints - 1 + retention_duration worth of attestations.
        // However, in case we happen to be adding a new attestation this block we allow for the clearing of 1 additional attestation.
        let max_attestations_to_remove = checkpoint_interval * 2 + retention_duration;

        let maybe_cursor =
            Attestations::<T>::clear_prefix(chain_key, max_attestations_to_remove, None)
                .maybe_cursor;
        ensure!(maybe_cursor.is_none(), Error::<T>::TooManyAttestations);

        CheckpointingQueues::<T>::remove(chain_key);
        AttestationRemovalQueues::<T>::remove(chain_key);

        // Get checkpoint digest for height
        let digest = Checkpoints::<T>::get(chain_key, checkpoint_height)
            .ok_or(Error::<T>::NoSuchCheckpoint)?;
        let checkpoint_data = AttestationCheckpoint {
            block_number: checkpoint_height,
            digest,
        };

        // Remove all entries with height > checkpoint_height from bucket containing `checkpoint_height`
        let checkpoint_pivot = Self::compute_block_index_for(checkpoint_height);
        let block_heights: Vec<u64> =
            CheckpointBuckets::<T>::iter_key_prefix((chain_key, checkpoint_pivot)).collect();
        for block_number in block_heights {
            if block_number > checkpoint_height {
                Checkpoints::<T>::remove(chain_key, block_number);
                CheckpointBuckets::<T>::remove((chain_key, checkpoint_pivot, block_number));
            }
        }

        let last_checkpoint = LastCheckpoint::<T>::get(chain_key);
        if let Some(checkpoint) = last_checkpoint {
            let pruning_state = CheckpointPruningState {
                stop_height: checkpoint.block_number,
                next_pivot: checkpoint_pivot.saturating_add(CHECKPOINT_BUCKET_SIZE),
            };
            // Set an initial pivot at which to begin clearing checkpoint buckets.
            // MAX_CHECKPOINTS_CLEARED_PER_BLOCK entries will be cleared per block
            // in on_initialize until all buckets above our revert height are cleared.
            CheckpointPruningStates::<T>::insert(chain_key, pruning_state);
        } else {
            return Err(Error::<T>::LastCheckpointNotSet.into());
        }

        // Set last digest and last checkpoint equal to `checkpoint_digest`
        LastCheckpoint::<T>::set(chain_key, Some(checkpoint_data));
        LastDigest::<T>::set(chain_key, Some((checkpoint_height, digest)));

        Ok(digest)
    }

    /// Helpers to keep on_initialize readable ///
    pub fn on_init_clear_checkpoints() -> bool {
        if let Some((chain_key, cursor)) = CheckpointClearingCursors::<T>::iter().next() {
            if let Some(cursor) = cursor.inner {
                let maybe_cursor = Checkpoints::<T>::clear_prefix(
                    chain_key,
                    u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                    Some(&cursor[..]),
                )
                .maybe_cursor;
                let maybe_cursor_struct = if maybe_cursor.is_some() {
                    Some(ClearingCursor::new(maybe_cursor))
                } else {
                    None
                };
                CheckpointClearingCursors::<T>::set(chain_key, maybe_cursor_struct);
            } else {
                if !cursor.is_benchmark {
                    // We have a clearing cursor with an empty inner cursor. We can ignore and remove it.
                    CheckpointClearingCursors::<T>::remove(chain_key);
                    return true;
                } else {
                    let maybe_cursor = Checkpoints::<T>::clear_prefix(
                        chain_key,
                        u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                        None,
                    )
                    .maybe_cursor;
                    let maybe_cursor_struct = if maybe_cursor.is_some() {
                        Some(ClearingCursor::new(maybe_cursor))
                    } else {
                        None
                    };
                    CheckpointClearingCursors::<T>::set(chain_key, maybe_cursor_struct);
                }
            };

            // note: may be triggered multiple times when removing a large amount of checkpoints
            Self::deposit_event(Event::<T>::CheckpointsCleared(chain_key));

            // Performed clear checkpoints operation (for gas calculation)
            true
        } else {
            false
        }
    }

    pub fn on_init_clear_buckets() -> bool {
        if let Some((chain_key, cursor)) = BucketClearingCursors::<T>::iter().next() {
            if let Some(cursor) = cursor.inner {
                let maybe_cursor = CheckpointBuckets::<T>::clear_prefix(
                    (chain_key,),
                    u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                    Some(&cursor[..]),
                )
                .maybe_cursor;
                let maybe_cursor_struct = if maybe_cursor.is_some() {
                    Some(ClearingCursor::new(maybe_cursor))
                } else {
                    None
                };
                BucketClearingCursors::<T>::set(chain_key, maybe_cursor_struct);
            } else {
                if !cursor.is_benchmark {
                    // We have a clearing cursor with an empty inner cursor. We can ignore and remove it.
                    BucketClearingCursors::<T>::remove(chain_key);
                    return true;
                } else {
                    let maybe_cursor = CheckpointBuckets::<T>::clear_prefix(
                        (chain_key,),
                        u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                        None,
                    )
                    .maybe_cursor;
                    let maybe_cursor_struct = if maybe_cursor.is_some() {
                        Some(ClearingCursor::new(maybe_cursor))
                    } else {
                        None
                    };
                    BucketClearingCursors::<T>::set(chain_key, maybe_cursor_struct);
                }
            };

            // performed clear buckets operation (for gas calculation)
            true
        } else {
            false
        }
    }

    pub fn on_init_prune_checkpoints() -> bool {
        match CheckpointPruningStates::<T>::iter().next() {
            Some((chain_key, state)) => {
                Self::prune_checkpoints_impl(chain_key, state);
                // performed prune checkpoints operation (for gas calculation)
                true
            }
            None => false,
        }
    }

    fn prune_checkpoints_impl(chain_key: ChainKey, mut state: CheckpointPruningState) {
        let mut remaining_entries = u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK);
        // In the extremely unlikely case that there are 1000's of pivots containing no entries, this
        // prevents us from looping and reading state until we max out the block's compute.
        let mut remaining_pivots = u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK * 2);

        while remaining_entries > 0 && remaining_pivots > 0 {
            let current_pivot = state.next_pivot;

            // 1) If state.next_pivot > state.stop_height, then we are done clearing checkpoints.
            if current_pivot > state.stop_height {
                CheckpointPruningStates::<T>::remove(chain_key);
                return; // We return here to prevent Checkpoint pruning states from being reset below
            }

            // 2) Clear as much of the pivot as we can this block

            // Get removal heights first, as it's unsafe to remove entries directly within the iterator
            // Iterating these keys is technically O(bucket_size), so we benchmark for the very pessimistic
            // case of 1 checkpoint every block.
            let removal_heights: sp_std::vec::Vec<u64> =
                CheckpointBuckets::<T>::iter_key_prefix((chain_key, current_pivot)).collect();
            let removal_limit = usize::min(removal_heights.len(), remaining_entries as usize);
            for idx in 0..removal_limit {
                if let Some(height) = removal_heights.get(idx) {
                    Checkpoints::<T>::remove(chain_key, height);
                    CheckpointBuckets::<T>::remove((chain_key, current_pivot, height));
                } else {
                    log::error!("Could not access removal_height. This shouldn't happen! Skipping remaining checkpoint clearing this block.");
                    return;
                }
            }

            remaining_entries = remaining_entries.saturating_sub(removal_limit as u32);

            // We were able to remove all entries in this pivot. Proceed to next
            if removal_heights.len() == removal_limit {
                state.next_pivot = current_pivot.saturating_add(CHECKPOINT_BUCKET_SIZE);
                remaining_pivots -= 1;
            }
        }

        // If we didn't finish, ensure state persisted
        CheckpointPruningStates::<T>::insert(chain_key, state);
    }
}

impl<T: Config> ChainRemovalListener for Pallet<T> {
    fn on_supported_chain_removed(chain_key: ChainKey, remove_checkpoints: bool) {
        Self::chill_all_attestors_for_chain(chain_key);

        ActiveAttestors::<T>::remove(chain_key);

        // Can dispense with result, since limit is equal to maximum storage size
        _ = Invulnerables::<T>::clear_prefix(
            chain_key,
            MaxInvulnerables::<T>::get(chain_key),
            None,
        );

        MaxAttestors::<T>::remove(chain_key);

        MaxInvulnerables::<T>::remove(chain_key);

        // Clearing attestations
        let retention_duration = AttestationRetentionDuration::<T>::get(chain_key);
        let max_attestations_to_remove =
            AttestationCheckpointInterval::<T>::get(chain_key) * 2 + retention_duration;
        // Can dispense with result, since limit is equal to maximum storage size
        _ = Attestations::<T>::clear_prefix(chain_key, max_attestations_to_remove, None);

        CheckpointingQueues::<T>::remove(chain_key);
        AttestationRemovalQueues::<T>::remove(chain_key);
        LastCheckpoint::<T>::remove(chain_key);
        LastDigest::<T>::remove(chain_key);
        PendingTargetSampleSize::<T>::remove(chain_key);
        TargetSampleSize::<T>::remove(chain_key);
        ChainAttestationInterval::<T>::remove(chain_key);
        PendingAttestationInterval::<T>::remove(chain_key);
        AttestationCheckpointInterval::<T>::remove(chain_key);
        MaxCatchup::<T>::remove(chain_key);
        PendingMaxCatchup::<T>::remove(chain_key);
        AttestationRetentionDuration::<T>::remove(chain_key);

        if remove_checkpoints {
            // Starting the process of clearing checkpoints. There may be a very large number of checkpoints
            // in storage, and we aren't in a huge hurry to clear them out. So we clear a moderate number per
            // block.
            let maybe_cursor = Checkpoints::<T>::clear_prefix(
                chain_key,
                u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                None,
            )
            .maybe_cursor;

            if maybe_cursor.is_some() {
                // more checkpoints left to be removed
                // Attestation pallet will check this storage to trigger further checkpoint removals in future blocks
                // and CheckpointsCleared event will be dispatched inside on_initialize()
                CheckpointClearingCursors::<T>::set(
                    chain_key,
                    Some(ClearingCursor::new(maybe_cursor)),
                );
            } else {
                // all checkpoints were removed in the call above, trigger the event here
                // b/c on_initialize() won't do that
                Self::deposit_event(Event::<T>::CheckpointsCleared(chain_key));
            }

            // Starting process of clearing checkpoint buckets. We use a similar procedure to that used with checkpoints.
            let maybe_buckets_cursor = CheckpointBuckets::<T>::clear_prefix(
                (chain_key,),
                u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                None,
            )
            .maybe_cursor;

            if maybe_buckets_cursor.is_some() {
                // more buckets left to be removed
                BucketClearingCursors::<T>::set(
                    chain_key,
                    Some(ClearingCursor::new(maybe_buckets_cursor)),
                );
            }
        }

        // If there is an ongoing chain reversion for the chain being removed, we can
        // drop the reversion in favor of the removal.
        CheckpointPruningStates::<T>::remove(chain_key);

        Self::deposit_event(Event::<T>::ClearedStorageForRemovedChain(chain_key));
    }
}
