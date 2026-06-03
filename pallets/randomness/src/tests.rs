use super::*;
use mock::*;
use randomness_primitives::provider::RandomnessPalletProvider;
use sp_core::H256;

#[test]
fn first_two_epoch_empty_randomness() {
    new_test_ext(1).execute_with(|| {
        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get(0),
            H256::zero().0
        );
        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get(1),
            H256::zero().0
        );
        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get(2),
            H256::zero().0
        );
    });
}

#[test]
fn can_predict_next_epoch_change() {
    new_test_ext(1).execute_with(|| {
        System::set_block_number(1);
        Timestamp::set_timestamp(1);

        let last_seen_epoch_index = crate::LastSeenEpochIndex::<Test>::get();
        assert_eq!(last_seen_epoch_index, 0);

        assert_eq!(<Test as pallet_babe::Config>::EpochDuration::get(), 3);
        // this sets the genesis slot to 6;
        go_to_block(1, 6);
        assert_eq!(*Babe::genesis_slot(), 6);
        assert_eq!(*Babe::current_slot(), 6);
        assert_eq!(Babe::epoch_index(), 0);

        progress_to_block(5);

        assert_eq!(Babe::epoch_index(), 5 / 3);
        assert_eq!(*Babe::current_slot(), 10);

        // RandomnessPallet::on_initialize(5);

        let last_seen_epoch_index = crate::LastSeenEpochIndex::<Test>::get();
        assert_eq!(last_seen_epoch_index, 1);

        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get(1),
            pallet_babe::Randomness::<Test>::get()
        );

        progress_to_block(7);

        assert_eq!(Babe::epoch_index(), 7 / 3);
        assert_eq!(*Babe::current_slot(), 12);

        let last_seen_epoch_index = crate::LastSeenEpochIndex::<Test>::get();
        assert_eq!(last_seen_epoch_index, 2);

        // assert on emited event
        System::assert_last_event(
            crate::Event::StoreRandomnessForEpoch {
                epoch_index: 2,
                randomness: crate::Pallet::<Test>::randomness_by_epoch_index(2),
            }
            .into(),
        );

        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get(2),
            pallet_babe::Randomness::<Test>::get()
        );

        // these two functions return the value from storage
        assert_eq!(
            crate::Pallet::<Test>::randomness_by_epoch_index(2),
            crate::Pallet::<Test>::randomness_by_epoch_id(2)
        );
    })
}

#[test]
fn old_epoch_randomness_is_pruned() {
    new_test_ext(1).execute_with(|| {
        System::set_block_number(1);
        Timestamp::set_timestamp(1);

        // Genesis slot = 6; EpochDuration = 3 slots; MaxEpochHistory = 3.
        // Each block advances the slot by 1, so epoch N starts at block 3N + 1.
        go_to_block(1, 6);
        assert_eq!(Babe::epoch_index(), 0);

        // We are still in epoch 0, so no pruning should be scheduled.
        assert_eq!(crate::PruningQueue::<Test>::get(), None);

        // Advance to epoch 4 (block 13, slot 18).
        progress_to_block(13);
        assert_eq!(Babe::epoch_index(), 4);

        // Entry 0 should have been pruned
        assert!(!crate::RandomnessByEpochIndex::<Test>::contains_key(0));

        // Entries for epochs 1, 2, 3, 4 must be present.
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(1));
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(2));
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(3));
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(4));

        // We are in epoch 4, so pruning should have been scheduled for epochs 0 and 1.
        assert_eq!(
            crate::PruningQueue::<Test>::get(),
            Some(crate::PruningState { next: 1, to: 1 })
        );

        progress_to_block(14);
        // Epoch 1 should have been pruned
        assert!(!crate::RandomnessByEpochIndex::<Test>::contains_key(1));
        // Entries for epochs 2, 3, 4 must be present.
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(2));
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(3));
        assert!(crate::RandomnessByEpochIndex::<Test>::contains_key(4));

        progress_to_block(15);
        // Prunning queu should have been cleared since we've pruned up to the target epoch.
        assert_eq!(crate::PruningQueue::<Test>::get(), None);
    })
}

mod migration {
    use super::*;
    use crate::migrations::MigratePruningQueueV0ToV1;
    use frame_support::traits::{
        GetStorageVersion, OnInitialize, OnRuntimeUpgrade, StorageVersion,
    };

    // MaxEpochHistory in the mock runtime.
    const MAX_HISTORY: u64 = 3;

    /// Simulate a chain that ran under storage version 0: a backlog of entries with no
    /// pruning queue, and the on-chain storage version still at 0.
    fn setup_v0_backlog(last_seen: u64) {
        for epoch in 0..=last_seen {
            crate::RandomnessByEpochIndex::<Test>::insert(epoch, H256::repeat_byte(1).0);
        }
        crate::LastSeenEpochIndex::<Test>::put(last_seen);
        crate::PruningQueue::<Test>::kill();
        StorageVersion::new(0).put::<crate::Pallet<Test>>();
    }

    #[test]
    fn seeds_pruning_queue_for_full_backlog() {
        new_test_ext(1).execute_with(|| {
            let last_seen = 100u64;
            setup_v0_backlog(last_seen);

            MigratePruningQueueV0ToV1::<Test>::on_runtime_upgrade();

            // Version bumped and queue seeded to span the whole backlog from epoch 0.
            assert_eq!(
                crate::Pallet::<Test>::on_chain_storage_version(),
                StorageVersion::new(1)
            );
            assert_eq!(
                crate::PruningQueue::<Test>::get(),
                Some(crate::PruningState {
                    next: 0,
                    to: last_seen - MAX_HISTORY,
                })
            );
        });
    }

    #[test]
    fn seeded_queue_drains_backlog_but_keeps_recent_history() {
        new_test_ext(1).execute_with(|| {
            let last_seen = 10u64;
            setup_v0_backlog(last_seen);

            MigratePruningQueueV0ToV1::<Test>::on_runtime_upgrade();

            // Drive the per-block pruning to completion. The queue removes one entry per
            // call; `on_initialize` also re-reads babe's epoch index — which stays at 0 in
            // this bare-storage fixture, so no new epoch is recorded and only pruning runs.
            for block in 1..=(last_seen + MAX_HISTORY + 1) {
                crate::Pallet::<Test>::on_initialize(block);
            }

            // Queue fully drained.
            assert_eq!(crate::PruningQueue::<Test>::get(), None);

            // Stale epochs (<= last_seen - MAX_HISTORY) are gone...
            for epoch in 0..=(last_seen - MAX_HISTORY) {
                assert!(
                    !crate::RandomnessByEpochIndex::<Test>::contains_key(epoch),
                    "stale epoch {epoch} should have been pruned"
                );
            }
            // ...and the most recent `MAX_HISTORY` epochs are retained.
            for epoch in (last_seen - MAX_HISTORY + 1)..=last_seen {
                assert!(
                    crate::RandomnessByEpochIndex::<Test>::contains_key(epoch),
                    "recent epoch {epoch} should have been retained"
                );
            }
        });
    }

    #[test]
    fn no_queue_when_history_within_retention_window() {
        new_test_ext(1).execute_with(|| {
            // last_seen < MaxEpochHistory: every recorded epoch is still within the
            // retention window, so `last_seen - max_history` underflows and nothing is
            // scheduled for pruning.
            setup_v0_backlog(MAX_HISTORY - 1);

            MigratePruningQueueV0ToV1::<Test>::on_runtime_upgrade();

            assert_eq!(
                crate::Pallet::<Test>::on_chain_storage_version(),
                StorageVersion::new(1)
            );
            // Version bumped, but no queue seeded.
            assert_eq!(crate::PruningQueue::<Test>::get(), None);
        });
    }

    #[test]
    fn is_idempotent_and_skips_when_already_migrated() {
        new_test_ext(1).execute_with(|| {
            let last_seen = 50u64;
            setup_v0_backlog(last_seen);

            // First run seeds the queue.
            MigratePruningQueueV0ToV1::<Test>::on_runtime_upgrade();
            // Mutate the queue as if some draining progress was made.
            crate::PruningQueue::<Test>::put(crate::PruningState { next: 20, to: 47 });

            // Second run must be a no-op: already at v1, so the queue is left untouched.
            MigratePruningQueueV0ToV1::<Test>::on_runtime_upgrade();

            assert_eq!(
                crate::PruningQueue::<Test>::get(),
                Some(crate::PruningState { next: 20, to: 47 })
            );
        });
    }
}
