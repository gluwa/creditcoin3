use super::*;
use mock::*;
use randomness_primitives::{provider::RandomnessPalletProvider, Randomness};
use sp_core::H256;

/// Mirror of the legacy (storage version 0) unbounded `StorageMap`, used to seed
/// pre-migration state in tests.
#[frame_support::storage_alias]
type RandomnessByEpochIndex<T: crate::Config> = StorageMap<
    crate::Pallet<T>,
    frame_support::Blake2_128Concat,
    u64,
    Randomness,
    frame_support::pallet_prelude::ValueQuery,
>;

#[test]
fn first_two_epoch_empty_randomness() {
    new_test_ext(1).execute_with(|| {
        assert_eq!(
            crate::Pallet::<Test>::randomness_by_epoch_id(0),
            H256::zero().0
        );
        assert_eq!(
            crate::Pallet::<Test>::randomness_by_epoch_id(1),
            H256::zero().0
        );
        assert_eq!(
            crate::Pallet::<Test>::randomness_by_epoch_id(2),
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
            crate::Pallet::<Test>::randomness_by_epoch_id(1),
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
                randomness: crate::Pallet::<Test>::randomness_by_epoch_id(2),
            }
            .into(),
        );

        assert_eq!(
            crate::Pallet::<Test>::randomness_by_epoch_id(2),
            pallet_babe::Randomness::<Test>::get()
        );

        // the stored map exposes the same value as the provider lookup
        assert_eq!(
            crate::RandomnessByEpochIndex::<Test>::get()
                .get(&2)
                .copied()
                .unwrap_or_default(),
            crate::Pallet::<Test>::randomness_by_epoch_id(2)
        );
    })
}

#[test]
fn evicts_oldest_when_full() {
    use frame_support::traits::Get;

    new_test_ext(1).execute_with(|| {
        let bound = <<Test as crate::Config>::MaxRandomnessEntries as Get<u32>>::get() as u64;

        // Insert more epochs than the bound allows, oldest-first.
        for epoch in 0..bound + 3 {
            crate::RandomnessByEpochIndex::<Test>::mutate(|map| {
                if map.len() >= bound as usize {
                    let oldest = map.keys().next().copied().unwrap();
                    map.remove(&oldest);
                }
                map.try_insert(epoch, [epoch as u8; 32]).unwrap();
            });
        }

        let map = crate::RandomnessByEpochIndex::<Test>::get();
        // Never exceeds the bound.
        assert_eq!(map.len() as u64, bound);
        // Only the latest `bound` epochs remain.
        for epoch in 0..3 {
            assert!(!map.contains_key(&epoch));
        }
        for epoch in 3..bound + 3 {
            assert_eq!(map.get(&epoch).copied(), Some([epoch as u8; 32]));
        }
    });
}

#[test]
fn migration_v0_to_v1_keeps_latest_n() {
    use crate::migrations::MigrateRandomnessByEpochIndexV0ToV1;
    use frame_support::traits::{Get, GetStorageVersion, OnRuntimeUpgrade};

    new_test_ext(1).execute_with(|| {
        let bound = <<Test as crate::Config>::MaxRandomnessEntries as Get<u32>>::get() as u64;
        let total = bound + 3;

        // Seed the legacy unbounded storage map.
        for epoch in 0..total {
            RandomnessByEpochIndex::<Test>::insert(epoch, [epoch as u8; 32]);
        }

        MigrateRandomnessByEpochIndexV0ToV1::<Test>::on_runtime_upgrade();

        // Storage version bumped.
        assert_eq!(crate::Pallet::<Test>::on_chain_storage_version(), 1);

        let map = crate::RandomnessByEpochIndex::<Test>::get();
        assert_eq!(map.len() as u64, bound);

        // The latest `bound` epochs are preserved; the oldest are dropped.
        for epoch in 0..(total - bound) {
            assert!(!map.contains_key(&epoch));
        }
        for epoch in (total - bound)..total {
            assert_eq!(map.get(&epoch).copied(), Some([epoch as u8; 32]));
        }

        // Legacy entries are gone.
        assert!(RandomnessByEpochIndex::<Test>::iter().next().is_none());
    });
}
