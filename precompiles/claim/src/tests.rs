use crate::{
    mock::{
        Account::{Alice, Precompile},
        *,
    },
    types::EvmClaim,
};

use precompile_utils::prelude::Address;
use precompile_utils::testing::*;
use sp_core::{H160, H256};
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

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::submit_claim {
                        claim: EvmClaim {
                            block_number: 1,
                            chain_id: 42,
                            tx_index: 123,
                            to: Address(alice),
                            from: Address(bob),
                            is_tx: false,
                            is_rx: true,
                        },
                    },
                )
                .execute_returns(true);

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
                        claim: EvmClaim {
                            block_number: 1,
                            chain_id: 42,
                            tx_index: 123,
                            to: Address(alice),
                            from: Address(bob),
                            is_tx: false,
                            is_rx: true,
                        },
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
