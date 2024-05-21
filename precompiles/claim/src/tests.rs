use crate::mock::{
    Account::{Alice, Precompile},
    *,
};

use precompile_utils::prelude::Address;
use precompile_utils::testing::*;
use prover_primitives::claim::{Claim, ClaimKind};
use sp_core::{H160, H256};
use sp_runtime::traits::Hash;
use std::str::from_utf8;

// No test of invalid selectors since we have a fallback behavior (deposit).
fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

#[test]
fn submit_claim_works() {
    let alice: H160 = Alice.into();
    let alice_account: H256 = Alice.into();

    let bob: H160 = Bob.into();

    let claim = Claim {
        block_number: 1,
        chain_id: 42,
        tx_index: 123,
        to: alice,
        from: bob,
        kind: ClaimKind::Rx,
    };
    let claim_hash = <Runtime as pallet_prover::Config>::Hashing::hash_of(&claim);

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::submit_claim {
                        block_number: claim.block_number,
                        chain_id: claim.chain_id,
                        tx_index: claim.tx_index,
                        to: Address(claim.to),
                        from: Address(claim.from),
                        is_tx: claim.kind == ClaimKind::Tx,
                        is_rx: claim.kind == ClaimKind::Rx,
                    },
                )
                .execute_returns(claim_hash);

            let alice: Account = alice_account.0.into();
            let alice_balance = Balances::usable_balance(alice);

            // 100 CTC was locked as a commitment to allow the prover to process the claim
            assert_eq!(alice_balance, 200);
        });
}

#[test]
fn submit_claim_fails_without_enough_balance() {
    let alice: H160 = Alice.into();

    let bob: H160 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 50)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::submit_claim {
                        block_number: 1,
                        chain_id: 42,
                        tx_index: 123,
                        to: Address(alice),
                        from: Address(bob),
                        is_tx: false,
                        is_rx: true,
                    },
                )
                .execute_reverts(|output| {
                    from_utf8(output)
                        .unwrap()
                        .contains("Dispatched call failed with error: ")
                        && from_utf8(output).unwrap().contains("BalanceToLow")
                });
        });
}

#[test]
fn submit_claim_and_proof_works() {
    let bob: H160 = Bob.into();
    let bob_account: H256 = Bob.into();

    let alice: H160 = Alice.into();
    let alice_account: H256 = Alice.into();

    let claim = Claim {
        block_number: 1,
        chain_id: 42,
        tx_index: 123,
        to: alice,
        from: bob,
        kind: ClaimKind::Rx,
    };

    let claim_hash = <Runtime as pallet_prover::Config>::Hashing::hash_of(&claim);

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (bob.into(), 101)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    bob,
                    Precompile,
                    PCall::submit_claim {
                        block_number: claim.block_number,
                        chain_id: claim.chain_id,
                        tx_index: claim.tx_index,
                        to: Address(claim.to),
                        from: Address(claim.from),
                        is_tx: claim.kind == ClaimKind::Tx,
                        is_rx: claim.kind == ClaimKind::Rx,
                    },
                )
                .execute_returns(claim_hash);

            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::usable_balance(bob);

            // 100 CTC was locked as a commitment to allow the prover to process the claim
            // Initial balance was 101
            assert_eq!(bob_balance, 1);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::submit_proof {
                        claim_hash,
                        proof: b"some_proof".to_vec(),
                    },
                )
                .execute_returns(true);

            let alice: Account = alice_account.0.into();
            let alice_balance = Balances::usable_balance(alice);

            // 100 CTC was locked as a commitment to allow the prover to process the claim
            assert_eq!(alice_balance, 400);
        });
}
