#![cfg(feature = "runtime-benchmarks")]
use super::*;
use crate::{types::BalanceFor, Config};
use frame_benchmarking::{account, benchmarks, whitelist_account};
use frame_support::traits::Currency;
use frame_system::RawOrigin;
use pallet_balances::Pallet as Balances;

use crate::types::Cc2BurnId;

benchmarks! {
    where_clause { where T: pallet_balances::Config<Balance = BalanceFor<T>> }
    add_authority {
        let origin = RawOrigin::Root;
        let who: T::AccountId = authority_account::<T>(true);
    }: _(origin, who)

    approve_collection {
        let sender = sender_account::<T>(true);
        Authorities::<T>::insert(sender.clone(), ());

        let origin = RawOrigin::Signed(sender);
        let burn_id = Cc2BurnId(100u64);
        let collector = collector_account::<T>(true);
        let min = <Balances<T> as Currency<T::AccountId>>::minimum_balance();
        let amount = min * 1_000_000u32.into();
    }: _(origin, burn_id, collector, amount)

    remove_authority {
        let origin = RawOrigin::Root;
        let who: T::AccountId = authority_account::<T>(true);
        Authorities::<T>::insert(who.clone(), ());

    }: _(origin, who)
}

//impl_benchmark_test_suite!(pallet_bridge, crate::mock::new_test_ext(), crate::mock::Test);

fn authority_account<T: Config>(whitelist: bool) -> T::AccountId {
    let authority = account("authority", 1, 1);
    if whitelist {
        whitelist_account!(authority);
    }
    authority
}

fn collector_account<T: Config>(whitelist: bool) -> T::AccountId {
    let collector = account("collector", 1, 1);
    if whitelist {
        whitelist_account!(collector);
    }
    collector
}

fn sender_account<T: Config>(whitelist: bool) -> T::AccountId {
    let sender = account("sender", 1, 1);
    if whitelist {
        whitelist_account!(sender);
    }
    sender
}
