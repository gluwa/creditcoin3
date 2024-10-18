use crate::{BalanceOf, Config};
use frame_support::traits::Currency;

/// Set balance that can be staked for `who`.
///
/// This includes any balance that is already staked.
#[cfg(any(test, feature = "runtime-benchmarks"))]
pub fn set_free_balance<T: Config>(who: &T::AccountId, value: BalanceOf<T>) {
    T::Currency::make_free_balance_be(who, value);
}
