#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::types::Prover;
use frame_benchmarking::{account, benchmarks};
use frame_support::traits::Currency;
use frame_support::traits::Get;
use frame_system::RawOrigin;
use prover_primitives::claim::ClaimKind;
use sp_runtime::SaturatedConversion;
use sp_std::vec;

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

    submit_claim {
        let who: T::AccountId = account("prover1", 1, 1);
        let prover : Prover = Prover {
            nickname: vec![1, 2, 3],
        };

        let claimer = account("claimer", 1, 1);

        let cash = T::Balance::saturated_from(200u64);

        <pallet_balances::Pallet<T> as Currency<T::AccountId>>::make_free_balance_be(&claimer, cash);

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

    submit_proof {

        let who: T::AccountId = account("prover1", 1, 1);
        let prover : Prover = Prover {
            nickname: vec![1, 2, 3],
        };

        let _ = Provers::<T>::clear(100, None);

        crate::Pallet::<T>::register_prover(RawOrigin::Signed(who.clone()).into(), prover.clone())?;
        assert_eq!(Provers::<T>::get(&who), Some(prover));

        let chain_price_config = vec![
            ChainPriceConfiguration {
                chain_id: 31337,
                price: 100,
            }
        ];

        crate::Pallet::<T>::set_chain_price_config(RawOrigin::Signed(who.clone()).into(), chain_price_config.clone())?;
        let claimer = account("claimer", 1, 1);


        let cash = T::Balance::saturated_from(chain_price_config[0].price);
        <pallet_balances::Pallet<T> as Currency<T::AccountId>>::make_free_balance_be(&claimer, cash);

        let claim = Claim {
            block_number: 1,
            chain_id: 31337,
            tx_index: 154,
            from: T::Address::default(),
            to: T::Address::default(),
            kind: ClaimKind::Tx,
        };

        crate::Pallet::<T>::submit_claim(RawOrigin::Signed(claimer.clone()).into(), claim.clone())?;

        use sp_runtime::traits::Hash;
        let claim_hash = <T as Config>::Hashing::hash_of(&claim);
        let proof = vec![1, 2, 3];

        assert_eq!(ClaimResultByHash::<T>::get(claim_hash), None);

    }: _(RawOrigin::Signed(who.clone()), claim_hash, proof)
    verify {
        assert_eq!(ClaimResultByHash::<T>::get(claim_hash), Some(true));
    }
}
