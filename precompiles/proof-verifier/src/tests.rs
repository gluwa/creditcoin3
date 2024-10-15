use crate::mock::{
    Account::{Alice, Bob, Precompile},
    *,
};

use pallet_prover_primitives::Query;
use precompile_utils::testing::*;
use sp_core::H160;
use std::str::from_utf8;

// No test of invalid selectors since we have a fallback behavior (deposit).
fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

#[test]
fn submit_proof_fails_proof_more_then_50_mb() {
    let bob: H160 = Bob.into();
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: 1,
        height: 1,
        index: 1,
        layout_segments: vec![],
        data: vec![],
    };

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (bob.into(), 101)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::verify {
                        proof: [0; 52428801].to_vec().into(), //52428801 is 50MB + 1 byte,
                        query,
                    },
                )
                .execute_reverts(|output| {
                    from_utf8(output)
                        .unwrap()
                        .contains("Value is too large for length")
                });
        });
}
