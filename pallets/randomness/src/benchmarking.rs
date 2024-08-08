#![cfg(feature = "runtime-benchmarks")]

use super::*;
use frame_benchmarking::v1::benchmarks;
use frame_support::traits::OnInitialize;
use sp_runtime::traits::One;

benchmarks! {
    on_initialize {
        frame_system::Pallet::<T>::set_block_number(One::one());
    }: {
        crate::Pallet::<T>::on_initialize(One::one());
    }
    verify {
        assert_eq!(LastSeenEpochIndex::<T>::get(), 1);
        assert_eq!(RandomnessByEpochIndex::<T>::get(1).unwrap(), pallet_babe::Randomness::<T>::get());
    }
}
