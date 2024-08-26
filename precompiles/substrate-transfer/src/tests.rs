use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    SELECTOR_LOG_TRANSFER,
};

use precompile_utils::{evm::logs::log3, solidity, testing::*};
use sp_core::{H160, H256, U256};
use std::str::from_utf8;

// No test of invalid selectors since we have a fallback behavior (deposit).
fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

#[test]
fn transfer_substrate_when_sender_has_enough_funds_should_work() {
    let alice: H160 = Alice.into();

    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // lock
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::transfer_substrate {
                        destination: bob_account,
                        amount: 200.into(),
                    },
                )
                // .execute_returns(true)
                .expect_log(log3(
                    Precompile,
                    SELECTOR_LOG_TRANSFER,
                    H256::from(alice),
                    bob_account,
                    solidity::encode_event_data(200_u128),
                ))
                .execute_returns(true);

            // Alice --
            let alice: Account = alice.into();
            let alice_balance = Balances::free_balance(alice);
            assert_eq!(alice_balance, 100);

            // Bob ++
            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 200);
        });
}

#[test]
fn transfer_substrate_when_sender_has_insufficient_funds_should_error() {
    let alice: H160 = Alice.into();

    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (Bob, 10)])
        .build()
        .execute_with(|| {
            // lock
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::transfer_substrate {
                        destination: bob_account,
                        amount: 400.into(),
                    },
                )
                .execute_reverts(|output| {
                    from_utf8(output)
                        .unwrap()
                        .contains("Dispatched call failed with error: ")
                        && from_utf8(output).unwrap().contains("Arithmetic(Underflow)")
                });

            // Alice - no change
            let alice: Account = alice.into();
            let alice_balance = Balances::free_balance(alice);
            assert_eq!(alice_balance, 300);

            // Bob - no change
            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 10);
        });
}

#[test]
fn transfer_substrate_when_amount_gt_max_balance_should_error() {
    let alice: H160 = Alice.into();

    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (Bob, 10)])
        .build()
        .execute_with(|| {
            // lock
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::transfer_substrate {
                        destination: bob_account,
                        // note: Substrate Balance is U128 but the argument here is U256!
                        amount: U256::MAX,
                    },
                )
                .execute_reverts(|output| {
                    // note: the Substrate call never gets dispatched in this case
                    from_utf8(output)
                        .unwrap()
                        .contains("Value is too large for balance type")
                });

            // Alice - no change
            let alice: Account = alice.into();
            let alice_balance = Balances::free_balance(alice);
            assert_eq!(alice_balance, 300);

            // Bob - no change
            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 10);
        });
}
