#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::types::Prover;
// use crate::mock::Balances;
use frame_benchmarking::{account, benchmarks, impl_benchmark_test_suite, whitelist_account};
use frame_system::RawOrigin;
use sp_std::vec;
// use crate::mock::ProverModule;
// use fp_account::AccountId20;
use prover_primitives::claim::ClaimKind;
use frame_support::traits::Currency;
use frame_support::traits::Get;

benchmarks! {
    register_prover {
        let who: T::AccountId = account("who", 1, 1);
        let prover : Prover = Prover {
            nickname: vec![1, 2, 3],
        };
    }: _(RawOrigin::Signed(who.clone()), prover.clone())
    verify {
        assert_eq!(Provers::<T>::get(&who), Some(prover));
    }

    set_chain_price_config {
        let who: T::AccountId = account("who", 1, 1);
        let prover : Prover = Prover {
            nickname: vec![1, 2, 3],
        };

        Provers::<T>::insert(&who, prover);
        let chain_price_config = vec![
            ChainPriceConfiguration {
                chain_id: 1,
                price: 100,
            }
        ];
    }: _(RawOrigin::Signed(who.clone()), chain_price_config.clone())
    verify {
        assert_eq!(ProversChainPriceConfigurations::<T>::get(&who), chain_price_config);
    }

    //todo probably need to combine this with the previous one
    // set_chain_price_config_2 {
    //     let who: T::AccountId = account("who", 1, 1);
    //     let prover : Prover = Prover {
    //         nickname: vec![1, 2, 3],
    //     };

    //     let chain_price_config = vec![
    //         ChainPriceConfiguration {
    //             chain_id: 1,
    //             price: 100,
    //         }
    //     ];
    //     Provers::<T>::insert(&who, prover);
    //     ProversChainPriceConfigurations::<T>::insert(&who, &chain_price_config);

    // }: set_chain_price_config(RawOrigin::Signed(who.clone()), vec![])
    // verify {
    //     assert_eq!(ProversChainPriceConfigurations::<T>::get(&who), vec![]);
    // }

    submit_claim {
        let who: T::AccountId = account("prover1", 1, 1);
        let prover : Prover = Prover {
            nickname: vec![1, 2, 3],
        };

        let chain_price_config = vec![
            ChainPriceConfiguration {
                chain_id: 31337,
                price: 100,
            }
        ];
        let claimer = account("claimer", 1, 1);

        let cash = T::MinBalance::get();

        <pallet_balances::Pallet<T> as Currency<T::AccountId>>::make_free_balance_be(&claimer,cash);

        let claim = Claim {
            block_number: 1,
            chain_id: 31337,
            tx_index: 154,
            from: T::Address::default(),
            to: T::Address::default(),
            kind: ClaimKind::Tx,
        };
        
    }: _(RawOrigin::Signed(claimer.clone()), claim.clone())
    verify {
        use sp_runtime::traits::Hash;
        let claim_hash = <T as Config>::Hashing::hash_of(&claim);
        assert_eq!(ClaimSourceByHash::<T>::get(claim_hash), Some(claimer));
    }
}

// impl_benchmark_test_suite!(
//     ProverModule,
//     crate::mock::ExtBuilder::build(Default::default()),
//     crate::mock::Test
// );
