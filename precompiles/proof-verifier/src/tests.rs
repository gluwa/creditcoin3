use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    BoundedBytes, ConstU50MB,
};

use frame_support::assert_ok;
use pallet_prover_primitives::{LayoutSegment, Query, ResultSegment, STARK_PROGRAM_V2_HASH};
use precompile_utils::testing::*;
use sp_core::H160;
use std::str::from_utf8;

#[cfg(target_arch = "x86_64")]
use sp_core::H256;

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

// exercises the scenario where the underlying extrinsic returns an error.
// had to change to return instead of a revert because it messes with the prover
// contract by consuming all the available gas
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
                .execute_returns((2u8, Vec::<ResultSegment>::new()));
        });
}

#[test]
fn verify_should_submit_error_code_when_stark_metadata_not_set() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: 31337,
        height: 1,
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 418, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
        }],
    };
    let proof_json = std::fs::read("../../cairo/stone-verifier/proof_example.json")
        .expect("Proof example not found");
    let proof: BoundedBytes<ConstU50MB> = proof_json.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::verify {
                        proof: proof.clone(),
                        query: query.clone(),
                    },
                )
                .execute_returns((4u8, Vec::<ResultSegment>::new()));
        });
}

// this test additionally logs an error since it's unable to verify the proof
#[cfg(all(test, target_arch = "x86_64"))]
#[test]
fn verify_should_return_error_code_when_proof_is_not_empty_but_not_valid() {
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
                        proof: b"abcd".to_vec().into(),
                        query,
                    },
                )
                .execute_returns((4u8, Vec::<ResultSegment>::new()));
        });
}

#[cfg(all(test, target_arch = "x86_64"))]
#[test]
fn submit_proof_should_error_when_stark_metadata_version_is_incorrect() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: 31337,
        height: 1,
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 418, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
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
                1,
                H256::random(),
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
                .execute_returns((4u8, Vec::<ResultSegment>::new()));
        });
}

// Exercises the scenario where the underlying extrinsic returns Ok()
// Only enabled for aarch64 until result segments are returned from all architectures.
#[cfg(all(test, target_arch = "aarch64"))]
#[test]
fn verify_should_return_zero_when_all_good() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: 31337,
        height: 1,
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 418, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
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
                .execute_returns((0u8, Vec::<ResultSegment>::new()));
            // TODO: Result segments returned in happy path vary depending on architecture.
            // Empty if not x86. This should change.
        });
}
