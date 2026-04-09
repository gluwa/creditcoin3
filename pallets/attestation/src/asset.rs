use crate::{BalanceOf, Config};
use frame_support::traits::Currency;

/// Existential deposit for the chain.
pub fn existential_deposit<T: Config>() -> BalanceOf<T> {
    T::Currency::minimum_balance()
}

/// Set balance that can be staked for `who`.
///
/// This includes any balance that is already staked.
#[cfg(feature = "runtime-benchmarks")]
pub fn set_free_balance<T: Config>(who: &T::AccountId, value: BalanceOf<T>) {
    T::Currency::make_free_balance_be(who, value);
}
