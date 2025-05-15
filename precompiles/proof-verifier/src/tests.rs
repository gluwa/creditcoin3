#[cfg(all(test, target_arch = "x86_64"))]
use crate::ResultSegmentsById;
use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    BoundedBytes, ConstU50MB,
};

use frame_support::assert_ok;
use pallet_attestation_poc::Attestations;
use pallet_prover::test_helpers::{create_dummy_attestation, PROOF_EXAMPLE_DIGEST_HEX};
use pallet_prover_primitives::{LayoutSegment, Query, STARK_PROGRAM_V3_HASH};
use precompile_utils::testing::*;
use sp_core::H160;
use sp_core::H256;
use std::str::from_utf8;

const SUPPORTED_CHAIN_KEY: u64 = 1;

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
        chain_id: SUPPORTED_CHAIN_KEY,
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
        chain_id: SUPPORTED_CHAIN_KEY,
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
                .execute_reverts(|r| r == b"Invalid proof submitted");
        });
}

#[test]
fn verify_should_revert_when_block_number_is_mismatched_between_query_and_the_proof() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: SUPPORTED_CHAIN_KEY,
        height: 6, // updated proof is generated for a query at block 4, we will set height to 6
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 681, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
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
                3,
                STARK_PROGRAM_V3_HASH
            ));

            let attestation = create_dummy_attestation(SUPPORTED_CHAIN_KEY, 10u64, None);
            let mut expected_digest = [0u8; 32];
            hex::decode_to_slice(PROOF_EXAMPLE_DIGEST_HEX, &mut expected_digest)
                .expect("example data is 32 bytes of valid hex");
            let h256_digest = H256::from(expected_digest);
            Attestations::<Runtime>::insert(SUPPORTED_CHAIN_KEY, h256_digest, attestation);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::verify {
                        proof: proof.clone(),
                        query: query.clone(),
                    },
                )
                .execute_reverts(|r| r == b"Checkpoint block number mismatch")
        });
}

// exercises the scenario where the underlying extrinsic returns Ok()
#[test]
fn verify_should_return_zero_when_all_good() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: SUPPORTED_CHAIN_KEY,
        height: 4, // updated proof is generated for a query at block 4
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 681, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
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
                3,
                STARK_PROGRAM_V3_HASH
            ));

            let attestation = create_dummy_attestation(SUPPORTED_CHAIN_KEY, 10u64, None);
            let mut expected_digest = [0u8; 32];
            hex::decode_to_slice(PROOF_EXAMPLE_DIGEST_HEX, &mut expected_digest)
                .expect("example data is 32 bytes of valid hex");
            let h256_digest = H256::from(expected_digest);
            Attestations::<Runtime>::insert(SUPPORTED_CHAIN_KEY, h256_digest, attestation);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::verify {
                        proof: proof.clone(),
                        query: query.clone(),
                    },
                )
                .execute_returns(0u8);
        });
}

#[test]
fn get_result_segments_should_error_for_unknown_query_id() {
    let alice: H160 = Alice.into();

    // note: not submitted on-chain therefore
    // no ResultSegments available for this query.id()
    let query = Query {
        chain_id: SUPPORTED_CHAIN_KEY,
        height: 4,
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 681, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
        }],
    };
    let query_id = query.id();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_result_segments { query_id })
                .execute_error(fp_evm::ExitError::Other(sp_std::borrow::Cow::Owned(
                    format!("Result segments not found for query: {:?}", query_id),
                )));
        });
}

#[test]
#[cfg(all(test, target_arch = "x86_64"))]
fn get_result_segments_should_work_for_known_query_id() {
    let alice: H160 = Alice.into();

    let query = Query {
        chain_id: SUPPORTED_CHAIN_KEY,
        height: 4,
        index: 0,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 681, // 418 / 31 + 418 % 31 != 0 = 14 (31 being `utils::utils::U248_BYTE_COUNT`)
        }],
    };
    let query_id = query.id();
    let proof_json = std::fs::read("../../cairo/stone-verifier/proof_example.json")
        .expect("Proof example not found");
    let proof: BoundedBytes<ConstU50MB> = proof_json.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            assert_ok!(ProverModule::set_stark_program_metadata(
                RuntimeOrigin::root(),
                3,
                STARK_PROGRAM_V3_HASH
            ));

            let attestation = create_dummy_attestation(SUPPORTED_CHAIN_KEY, 10u64, None);
            let mut expected_digest = [0u8; 32];
            hex::decode_to_slice(PROOF_EXAMPLE_DIGEST_HEX, &mut expected_digest)
                .expect("example data is 32 bytes of valid hex");
            let h256_digest = H256::from(expected_digest);
            Attestations::<Runtime>::insert(SUPPORTED_CHAIN_KEY, h256_digest, attestation);

            assert_ok!(ProverModule::submit_proof(
                RuntimeOrigin::signed(Alice),
                proof.into(),
                query
            ));

            // read the expected value from chain storage
            let result_segments: Option<_> = ResultSegmentsById::<Runtime>::get(query_id);
            let expected_segments = Vec::from(result_segments.unwrap());

            precompiles()
                .prepare_test(alice, Precompile, PCall::get_result_segments { query_id })
                .execute_returns(expected_segments);
        });
}
