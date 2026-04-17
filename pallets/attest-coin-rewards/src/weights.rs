//! Weights for `pallet-attest-coin-rewards` (hand-tuned; replace with benchmark output when tuning for mainnet).

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::weights::Weight;
use core::marker::PhantomData;

/// Weight functions for `pallet_attest_coin_rewards`.
pub struct WeightInfo<T>(PhantomData<T>);
impl<T: frame_system::Config> crate::WeightInfo for WeightInfo<T> {
    fn set_attest_coin_token() -> Weight {
        Weight::from_parts(25_000, 0)
    }
}
