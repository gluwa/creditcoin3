//! Reserve block assembly independently of EVM execution and gas fees.
//!
//! The committed coefficients were measured by compiled Wasm on AMD EPYC 7713,
//! 2026-10-13. Each input consumes gas: transactions >=21k, LOG0 >=375,
//! LOG4 >=1875, 1 KiB LOG0 data >=8567, zero calldata >=4/byte, and a distinct
//! storage slot >=2000 (access-list slot plus a warm unchanged SSTORE).
//! Take the highest marginal cost per gas, allow a 20% gas refund (5/4),
//! then add 25% measurement headroom (5/4). Storage writes inside the state-root
//! fixture are excluded: execution already pays for them. The shallow test trie
//! does not replace measuring against a deployed network's state.

use core::marker::PhantomData;
use frame_support::{traits::Get, weights::Weight};

pub mod full_block;
pub mod single_component;

#[cfg(test)]
mod tests;

pub struct WeightInfo<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo<T> {
    pub fn ref_time_per_gas() -> u64 {
        use single_component::WeightInfo as Bench;
        let cases = [
            (
                Bench::<T>::on_finalize_transactions(1)
                    .saturating_sub(Bench::<T>::on_finalize_transactions(0)),
                21_000,
            ),
            (
                Bench::<T>::on_finalize_logs(1).saturating_sub(Bench::<T>::on_finalize_logs(0)),
                375,
            ),
            (
                Bench::<T>::on_finalize_topics(1).saturating_sub(Bench::<T>::on_finalize_topics(0)),
                1_875,
            ),
            (
                Bench::<T>::on_finalize_log_data(1)
                    .saturating_sub(Bench::<T>::on_finalize_log_data(0)),
                8_567,
            ),
            (
                Bench::<T>::on_finalize_transaction_size(1)
                    .saturating_sub(Bench::<T>::on_finalize_transaction_size(0)),
                4_096,
            ),
            (
                Bench::<T>::on_finalize_state_root(1)
                    .saturating_sub(Bench::<T>::on_finalize_state_root(0))
                    .saturating_sub(T::DbWeight::get().writes(1)),
                2_000,
            ),
        ];
        cases
            .into_iter()
            .map(|(weight, gas)| weight.ref_time().div_ceil(gas))
            .max()
            .unwrap_or_default()
            .saturating_mul(25)
            .div_ceil(16)
    }
}

impl<T: frame_system::Config> pallet_ethereum::FinalizationWeight for WeightInfo<T> {
    fn base() -> Weight {
        use single_component::WeightInfo as Bench;
        let measured = [
            Bench::<T>::on_finalize_transactions(0),
            Bench::<T>::on_finalize_logs(0),
            Bench::<T>::on_finalize_topics(0),
            Bench::<T>::on_finalize_log_data(0),
            Bench::<T>::on_finalize_transaction_size(0),
            Bench::<T>::on_finalize_state_root(0),
        ]
        .into_iter()
        .fold(Weight::zero(), |maximum, weight| maximum.max(weight));
        Weight::from_parts(
            measured.ref_time().saturating_mul(5).div_ceil(4),
            measured.proof_size().saturating_mul(5).div_ceil(4),
        )
    }

    fn transaction(gas: u64) -> Weight {
        // At most 1/2 proof byte per gas, including the same refund/headroom factors.
        // This covers the largest measured slope (2094 bytes / 8567 gas).
        Weight::from_parts(
            Self::ref_time_per_gas().saturating_mul(gas),
            gas.div_ceil(2),
        )
    }
}
