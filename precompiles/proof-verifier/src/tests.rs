use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    BoundedBytes, ConstU50MB,
};
use pallet_prover_primitives::Query;
use precompile_utils::testing::*;
use sp_core::H160;
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
fn verify_should_error_when_stark_metadata_not_set() {
    let alice: H160 = Alice.into();
    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            System::set_block_number(1);

            // using some random incorrect proof because the verification will error out at
            // metadata not set before reaching the proof part
            let proof = BoundedBytes::<ConstU50MB>::from(vec![0; 10]);

            let query = Query {
                chain_id: SUPPORTED_CHAIN_KEY,
                height: 1,
                index: 1,
                layout_segments: vec![],
            };

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::verify {
                        proof: proof.clone(),
                        query: query.clone(),
                    },
                )
                .execute_reverts(|r| r == b"Stark program metadata not set")
        })
}

#[cfg(all(test, target_arch = "x86_64"))]
mod arch_dependent_tests {
    use super::*;
    use crate::VerifyResult;
    use frame_support::assert_ok;
    use pallet_attestation_poc::Attestations;
    use pallet_prover_primitives::{
        get_test_query, LayoutSegment, ResultSegment, STARK_PROGRAM_V2_HASH, STARK_PROGRAM_V3_HASH,
    };
    use sp_core::H256;

    #[test]
    fn verify_should_revert_when_block_number_is_mismatched_between_query_and_the_proof() {
        let alice: H160 = Alice.into();

        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![
                LayoutSegment {
                    offset: 448,
                    size: 32,
                },
                LayoutSegment {
                    offset: 192,
                    size: 32,
                },
                LayoutSegment {
                    offset: 224,
                    size: 32,
                },
                LayoutSegment {
                    offset: 800,
                    size: 32,
                },
                LayoutSegment {
                    offset: 928,
                    size: 32,
                },
                LayoutSegment {
                    offset: 960,
                    size: 32,
                },
                LayoutSegment {
                    offset: 992,
                    size: 32,
                },
                LayoutSegment {
                    offset: 1056,
                    size: 32,
                },
            ],
        };
        let proof_json = std::fs::read("../../cairo/stone-verifier/proof_example_erc20.json")
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
                    .execute_reverts(|r| r == b"Continuity proof block number mismatch")
            });
    }

    // exercises the scenario where the underlying extrinsic returns Ok()
    #[test]
    fn verify_should_return_zero_when_all_good() {
        let alice: H160 = Alice.into();

        let query = get_test_query();
        let proof_json = std::fs::read("../../cairo/stone-verifier/proof_example_erc20.json")
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

                let attestation = create_dummy_attestation(SUPPORTED_CHAIN_KEY, 30u64, None);
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
                    .execute_returns(VerifyResult {
                        status: 0, // Success
                        result_segments: vec![
                            ResultSegment { offset: 448, bytes: H256::from_slice(&hex::decode("0000000000000000000000000000000000000000000000000000000000000001").expect("Decoding failed"))},
                            ResultSegment { offset: 192, bytes: H256::from_slice(&hex::decode("000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266").expect("Decoding failed")) },
                            ResultSegment { offset: 224, bytes: H256::from_slice(&hex::decode("0000000000000000000000005fbdb2315678afecb367f032d93f642f64180aa3").expect("Decoding failed")) },
                            ResultSegment { offset: 800, bytes: H256::from_slice(&hex::decode("0000000000000000000000005fbdb2315678afecb367f032d93f642f64180aa3").expect("Decoding failed")) },
                            ResultSegment { offset: 928, bytes: H256::from_slice(&hex::decode("ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef").expect("Decoding failed")) },
                            ResultSegment { offset: 960, bytes: H256::from_slice(&hex::decode("000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266").expect("Decoding failed")) },
                            ResultSegment { offset: 992, bytes: H256::from_slice(&hex::decode("0000000000000000000000000000000000000000000000000000000000000001").expect("Decoding failed")) },
                            ResultSegment { offset: 1056, bytes: H256::from_slice(&hex::decode("0000000000000000000000000000000000000000000000000000000000000032").expect("Decoding failed")) }],
                    });
            });
    }
    // this test additionally logs an error since it's unable to verify the proof
    #[test]
    fn verify_should_error_when_proof_is_not_empty_but_not_valid() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                System::set_block_number(1);

                assert_ok!(ProverModule::set_stark_program_metadata(
                    RuntimeOrigin::root(),
                    2,
                    STARK_PROGRAM_V2_HASH
                ));

                let proof = BoundedBytes::<ConstU50MB>::from(b"abcd".to_vec());
                let query = Query {
                    chain_id: SUPPORTED_CHAIN_KEY,
                    height: 1,
                    index: 1,
                    layout_segments: vec![],
                };

                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::verify {
                            proof: proof.clone(),
                            query: query.clone(),
                        },
                    )
                    .execute_reverts(|r| r == b"Proof verification failed: ProcessError")
            })
    }

    #[test]
    fn verify_should_error_when_stark_metadata_version_is_incorrect() {
        let alice: H160 = Alice.into();
        ExtBuilder::default()
            .with_balances(vec![(alice.into(), 300)])
            .build()
            .execute_with(|| {
                System::set_block_number(1);

                assert_ok!(ProverModule::set_stark_program_metadata(
                    RuntimeOrigin::root(),
                    1,
                    H256::random(),
                ));

                let proof_json =
                    std::fs::read("../../cairo/stone-verifier/proof_example_erc20.json")
                        .expect("Proof example not found");
                let proof: BoundedBytes<ConstU50MB> = proof_json.into();

                let query = Query {
                    chain_id: SUPPORTED_CHAIN_KEY,
                    height: 1,
                    index: 1,
                    layout_segments: vec![],
                };

                precompiles()
                    .prepare_test(
                        alice,
                        Precompile,
                        PCall::verify {
                            proof: proof.clone(),
                            query: query.clone(),
                        },
                    )
                    .execute_reverts(|r| r == b"Proof verification failed: StarkMetadataMismatch")
            })
    }

    // Helper contents necessary to simulate attestations on-chain matching proof verification results
    use attestor_primitives::{Attestation as AttestationPrimitive, ChainKey, SignedAttestation};
    use sp_std::vec::Vec;

    pub const PROOF_EXAMPLE_DIGEST_HEX: &str =
        "0032e15872b4b900be9a24495f460b6b0114be936f80df5210d46d949abed889";

    pub fn create_dummy_attestation<AccountId>(
        chain_key: ChainKey,
        header_number: u64,
        prev_digest: Option<H256>,
    ) -> SignedAttestation<H256, AccountId> {
        let attestation = AttestationPrimitive {
            chain_key,
            header_number,
            header_hash: H256::zero(),
            root: [0; 32],
            prev_digest,
        };

        SignedAttestation {
            attestation,
            signature: [0u8; 96],
            attestors: Vec::new(),
            continuity_proof: Default::default(),
        }
    }
}
