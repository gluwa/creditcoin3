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
