use super::*;
use mock::*;
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

        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get(2),
            pallet_babe::Randomness::<Test>::get()
        );
    })
}
