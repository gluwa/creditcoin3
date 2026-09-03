use crate::{BalanceOf, Config};
use frame_support::traits::{tokens::fungibles::Inspect, Get};

/// Minimum balance for the bond asset account (Attest Coin).
pub fn existential_deposit<T: Config>() -> BalanceOf<T> {
    T::BondFungibles::minimum_balance(T::BondAssetId::get())
}

/// Set native balance that can be used for operational transfers (benchmarks).
#[cfg(feature = "runtime-benchmarks")]
pub fn set_free_balance<T: Config>(who: &T::AccountId, value: BalanceOf<T>) {
    use frame_support::traits::Currency;
    T::NativeCurrency::make_free_balance_be(who, value);
}

/// Set bond-asset balance for benchmarks (same units as [`Config::CurrencyBalance`]).
#[cfg(feature = "runtime-benchmarks")]
pub fn set_bond_balance<T: Config>(who: &T::AccountId, value: BalanceOf<T>) {
    use frame_support::traits::{tokens::fungibles::Mutate, Get};
    let id = T::BondAssetId::get();
    let _ = T::BondFungibles::set_balance(id, who, value);
}
