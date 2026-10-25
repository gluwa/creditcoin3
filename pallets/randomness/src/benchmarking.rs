use super::*;
use frame_benchmarking::v1::benchmarks;
use frame_support::traits::{Get, OnInitialize};
use sp_runtime::traits::One;

benchmarks! {
    // Common case: the epoch has not advanced, so `on_initialize` only performs
    // the two reads used to detect a new epoch and returns early.
    on_initialize {
        frame_system::Pallet::<T>::set_block_number(One::one());
    }: {
        crate::Pallet::<T>::on_initialize(One::one());
    }

    // Epoch boundary, pallet-local work only: write `LastSeenEpochIndex`, insert
    // into a *full* bounded map (forcing the worst-case eviction of the oldest
    // entry), and deposit the event. This deliberately measures
    // `store_epoch_randomness` rather than the full `on_initialize`, so the
    // configured `EventListeners` cost is NOT folded in here; that is accounted
    // separately via `OnRandomnessUpdateWeight` (the listener reports its own
    // benchmarked weight). Benchmarking the full `on_initialize` would instead
    // run the listener against an empty attestor set and under-count it.
    on_initialize_epoch_change {
        // Fill the bounded randomness map so the insert below must evict the
        // oldest entry (worst case for `RandomnessByEpochIndex::mutate`).
        let max = T::MaxRandomnessEntries::get();
        RandomnessByEpochIndex::<T>::mutate(|map| {
            for i in 0..max as u64 {
                let _ = map.try_insert(i, [i as u8; randomness_primitives::RANDOMNESS_LENGTH]);
            }
        });

        let epoch_index = u64::from(max) + 1;
    }: {
        crate::Pallet::<T>::store_epoch_randomness(
            epoch_index,
            [0u8; randomness_primitives::RANDOMNESS_LENGTH],
        );
    }
}
