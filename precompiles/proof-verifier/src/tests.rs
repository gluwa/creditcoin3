use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    BoundedBytes, ConstU50MB, SELECTOR_LOG_PROOF_SUBMITTED,
};

use frame_support::assert_ok;
use pallet_prover_primitives::{LayoutSegment, Query, STARK_PROGRAM_V2_HASH};
use precompile_utils::{evm::logs::log3, solidity, testing::*};
use sp_core::{H160, H256};
use std::str::from_utf8;

// No test of invalid selectors since we have a fallback behavior (deposit).
fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

// exercises the scenario where input data is invalid
#[test]
fn verify_should_revert_when_proof_larger_than_50_mb() {
    let alice: H160 = Alice.into();
    let bob: H160 = Bob.into();

    let query = Query {
        chain_id: 1,
        height: 1,
        index: 1,
        layout_segments: vec![],
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

// exercises the scenario where the underlying extrinsic returns an error
#[test]
fn verify_should_revert_when_proof_is_empty() {
    let alice: H160 = Alice.into();
    let bob: H160 = Bob.into();

    let query = Query {
        chain_id: 31337,
        height: 1,
        index: 0,
        layout_segments: vec![],
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
                        proof: b"".to_vec().into(),
                        query,
                    },
                )
                .execute_reverts(|output| {
                    from_utf8(output)
                        .unwrap()
                        .contains("Dispatched call failed with error: ")
                        && from_utf8(output).unwrap().contains("InvalidProofSubmitted")
                });
        });
}

// exercises the scenario where the underlying extrinsic returns Ok()
#[test]
fn verify_should_return_zero_when_all_good() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: 31337,
        height: 1,
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 14, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
        }],
    };
    let proof_json = std::fs::read("../../cairo/stone-verifier/proof_example.json")
        .expect("Proof example not found");
    let proof: BoundedBytes<ConstU50MB> = proof_json.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            assert_ok!(ProverModule::set_stark_program_metadata(
                RuntimeOrigin::root(),
                2,
                STARK_PROGRAM_V2_HASH
            ));

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::verify {
                        proof: proof.clone(),
                        query: query.clone(),
                    },
                )
                .expect_log(log3(
                    Precompile,
                    SELECTOR_LOG_PROOF_SUBMITTED,
                    H256::from(alice),
                    query.id(),
                    solidity::encode_event_data(proof),
                ))
                .execute_returns(0u8);
        });
}
