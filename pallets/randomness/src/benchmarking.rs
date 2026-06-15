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
}
