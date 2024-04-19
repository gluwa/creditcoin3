use crate::mock::{
    Account::{Alice, Precompile},
    *,
};

use precompile_utils::testing::*;
use sp_core::{H160, H256};

// No test of invalid selectors since we have a fallback behavior (deposit).
fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

#[test]
fn transfer() {
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
                .execute_returns(true);

            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 200);
        });
}
