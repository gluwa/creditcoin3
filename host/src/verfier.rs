use log::{debug, error};
use sp_core::H256;
use std::{
    cmp::Ordering,
    collections::HashMap,
    env, fs,
    io::Write,
    process::{Command, Stdio},
};
use tempfile::NamedTempFile;

use pallet_prover_primitives::{ContinuityProofLength, LayoutSegment, Query, ResultSegment};
use prover_primitives::{
    query::QueryValidationError::{self, *},
    stark_program_auth::{
        StarkProgramAuth, StarkProgramAuthHash, StarkProgramMetadata, StarkProgramMetadataStorage,
    },
    types::{CairoVerifierOutput, StoneProof, StoneProofJson},
};
use utils::utils::felts_from_bytes;

use crate::{error::VerifierError, result_segments};

/// The ABI encoded empty data (used to mean "null value").
pub const NULL_ABI: [u8; 1] = [0x80; 1];

pub fn run_verifier(
    proof: Vec<u8>,
    query: Query,
    metadata: Vec<(u8, StarkProgramAuthHash)>,
) -> Result<(String, Vec<ResultSegment>, ContinuityProofLength, H256), VerifierError> {
    debug!("current dir: {:?}", env::current_dir()?.as_os_str());

    // Write proof to a temporary JSON file
    let temp_file_path = write_proof_to_temp_file(&proof)?;

    debug!("Created temp file with proof at: {temp_file_path}");

    let proof: StoneProofJson = serde_json::from_slice(&proof)?;

    let mut stone_proof = StoneProof::from(proof.clone());

    stone_proof
        .strip_off_annotations()
        .strip_off_prover_config()
        .strip_off_private_input();

    // Last version is the highest version in the metadata
    let last_version = metadata.last().map(|(v, _)| *v).unwrap_or(0);
    // Prepare cairo program metadata
    let map: HashMap<StarkProgramAuthHash, StarkProgramMetadata> = metadata
        .into_iter()
        .map(|(k, v)| {
            (
                v as StarkProgramAuthHash,
                StarkProgramMetadata { version: k },
            )
        })
        .collect();

    let program_metadata_storage = StarkProgramMetadataStorage { map, last_version };

    // Authenticate the STARK program
    let metadata = StarkProgramAuth::authenticate(
        &stone_proof,
        &program_metadata_storage,
        blake2_256_stark_program_auth_hasher,
    )?;

    let cairo_verifier_output =
        CairoVerifierOutput::try_from(stone_proof.proof()).map_err(|e| {
            error!("Failed to convert StoneProof to CairoVerifierOutput: {e:?}",);
            VerifierError::CairoVerifierOutputConversionError(e)
        })?;

    // Save layout segments for later composition of ResultSegments
    let unsanitized_layout_segments = query.layout_segments.clone();

    validate_layout_segments(&unsanitized_layout_segments)?;

    // Sanitized layout segments are used to generate the layout segments hash in
    // verify_merkle_proof.cairo. So we validate using sanitized segments here as well.
    match validate_query_against_proof(query, &cairo_verifier_output) {
        Ok(_) => debug!("Query validated successfully"),
        Err(e) => return Err(VerifierError::QueryValidationError(e)),
    }

    debug!("stark program authenticated with metadata: {metadata:?}");

    // Execute the verifier command
    // WARNING: binary must be in $PATH and/or $PATH must be configured accordingly
    let output = Command::new("cpu_air_verifier")
        .arg(format!("--in_file={temp_file_path}"))
        .stdout(Stdio::piped())
        .output()?;

    fs::remove_file(&temp_file_path)?;

    let continuity_checkpoint_digest = H256::from(
        cairo_verifier_output
            .continuity_checkpoint_digest
            .to_bytes_be(),
    );
    if output.status.success() {
        // Return result segments along with message on success
        let query_felts = cairo_verifier_output.query_fields.clone();
        let result_segments: Vec<ResultSegment> =
            result_segments::get(&query_felts, &unsanitized_layout_segments)?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok((
            stdout,
            result_segments,
            cairo_verifier_output.continuity_proof_length,
            continuity_checkpoint_digest,
        ))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(VerifierError::VerifierProcessError(stderr))
    }
}

fn write_proof_to_temp_file(proof: &[u8]) -> Result<String, VerifierError> {
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(proof)?;
    let (_f, path) = temp_file.keep()?;

    let temp_file_path = path.to_str().ok_or(VerifierError::TempFileNotFound)?;

    Ok(temp_file_path.to_string())
}

fn blake2_256_stark_program_auth_hasher(bytes: &[u8]) -> StarkProgramAuthHash {
    H256::from(sp_io::hashing::blake2_256(bytes))
}

pub fn validate_query_against_proof(
    query: Query,
    cairo_verifier_output: &CairoVerifierOutput,
) -> Result<(), QueryValidationError> {
    match query.index.cmp(&cairo_verifier_output.query_index) {
        Ordering::Greater => Err(QueryOutOfBounds(cairo_verifier_output.query_index)),

        Ordering::Equal => {
            if felts_from_bytes(&NULL_ABI[..]) == cairo_verifier_output.query_fields {
                Err(QueryOutOfBounds(cairo_verifier_output.query_index))
            } else {
                // Sanitized layout segments are used to generate the layout segments hash in
                // verify_merkle_proof.cairo. So we validate using sanitized segments here as well.
                // Convert byte-based segments into felt-based offsets and sizes (31-byte alignment)
                // then sanitize them
                let sanitized_felts =
                    result_segments::convert_to_felts_then_sanitize(&query.layout_segments);
                let query = Query {
                    layout_segments: sanitized_felts,
                    ..query
                };
                debug!(
                    "Verifying layout segments hash. Sanitized felt segments: {:?}",
                    query.layout_segments
                );

                let local_offset_hash = match result_segments::hash_layout_segments(&query) {
                    Ok(hash) => hash,
                    Err(e) => {
                        error!("Failed to hash layout segments: {e:?}");
                        return Err(FailedToHashLayoutsegments(e.to_string()));
                    }
                };

                if local_offset_hash != cairo_verifier_output.query_hash {
                    Err(QueryOffsetsMismatch(
                        cairo_verifier_output.query_hash,
                        local_offset_hash,
                    ))
                } else {
                    Ok(())
                }
            }
        }

        Ordering::Less => Err(QueryTransactionIdMismatch(
            query.index,
            cairo_verifier_output.query_index,
        )),
    }
}

fn validate_layout_segments(layout_segments: &[LayoutSegment]) -> Result<(), VerifierError> {
    if layout_segments.is_empty() {
        return Err(VerifierError::QueryValidationError(
            QueryValidationError::QueryLayoutSegmentsError(
                "Layout segments cannot be empty".to_string(),
            ),
        ));
    }

    // Check that all segments have a non-zero byte size
    for segment in layout_segments {
        if segment.size == 0 {
            return Err(VerifierError::QueryValidationError(
                QueryValidationError::QueryLayoutSegmentsError(
                    "Layout segments must have a non-zero byte size".to_string(),
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod arch_independent_tests {
    use crate::verfier::VerifierError;
    use pallet_prover_primitives::LayoutSegment;

    #[test]
    fn should_validate_layout_segments_with_correct_segments() {
        let layout_segments = vec![
            LayoutSegment {
                offset: 0,
                size: 32,
            },
            LayoutSegment {
                offset: 32,
                size: 32,
            },
        ];
        let result = super::validate_layout_segments(&layout_segments);
        assert!(result.is_ok());
    }

    #[test]
    fn should_validate_layout_segments_with_correct_segments_non_32_bytes() {
        let layout_segments = vec![
            LayoutSegment { offset: 0, size: 2 },
            LayoutSegment {
                offset: 4,
                size: 33,
            },
        ];
        let result = super::validate_layout_segments(&layout_segments);
        assert!(result.is_ok());
    }

    #[test]
    fn should_error_on_empty_layout_segments() {
        let layout_segments: Vec<LayoutSegment> = vec![];
        let result = super::validate_layout_segments(&layout_segments);
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            VerifierError::QueryValidationError(
                super::QueryValidationError::QueryLayoutSegmentsError(_)
            )
        ));
    }

    #[test]
    fn should_error_on_invalid_layout_segment_size() {
        let layout_segments = vec![LayoutSegment {
            offset: 0,
            size: 0, // Invalid size
        }];
        let result = super::validate_layout_segments(&layout_segments);
        assert!(result.is_err());
        assert!(matches!(
            result.err().unwrap(),
            VerifierError::QueryValidationError(
                super::QueryValidationError::QueryLayoutSegmentsError(_)
            )
        ));
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
pub mod tests {
    use super::{felts_from_bytes, validate_query_against_proof, VerifierError, NULL_ABI};
    use log::error;
    use pallet_prover_primitives::{
        get_test_query, LayoutSegment, Query, ResultSegment, STARK_PROGRAM_V1_HASH,
        STARK_PROGRAM_V3_HASH,
    };
    use prover_primitives::{
        stark_program_auth::StarkProgramAuthError,
        types::{CairoVerifierOutput, StoneProof, StoneProofJson},
    };
    use sp_core::H256;

    #[test]
    fn verifying_authenticated_proof_should_return_ok() {
        let proof_path = "../cairo/stone-verifier/proof_example_erc20.json";

        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = get_test_query();

        let metadata = vec![(1, STARK_PROGRAM_V3_HASH)];

        let result = super::run_verifier(proof_example, query, metadata);

        assert!(result.is_ok());
    }

    #[test]
    fn verifying_authenticated_proof_should_return_correct_result_segments() {
        let proof_path = "../cairo/stone-verifier/proof_example_erc20.json";

        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = get_test_query();

        let metadata = vec![(1, STARK_PROGRAM_V3_HASH)];

        let result =
            super::run_verifier(proof_example, query, metadata).expect("Result should be Ok()");

        check_result_segments_against_expected(result.1);
    }

    #[test]
    fn verifying_stark_proof_with_bogus_public_memory_section_should_error() {
        // note: in this file the first 10 records in public_memory section have been altered
        // to 0x444 which should produce a different program hash and thus simulate
        // a STARK proof produced by an unauthorized/unauthenticated Cairo program
        // see StoneProof::program_bytes() and PublicInput::program_bytes() +
        // StarkProgramAuth::authenticate() for how the program hash is calculated!
        let proof_path = "../cairo/stone-verifier/bogus_public_memory_example.json";
        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = get_test_query();

        let metadata = vec![(1, STARK_PROGRAM_V3_HASH)];

        let result = super::run_verifier(proof_example, query, metadata);

        // Note that the program hash provided in the error message is the one coming from
        // the proof itself which is none of the existing hashes defined in the constants
        assert!(result.is_err());

        let error = result.err().unwrap();

        match error {
            VerifierError::StarkProgramAuthError(e) => {
                assert_eq!(
                    e,
                    StarkProgramAuthError::AuthenticationFailure(
                        "0x2a9480cea28d8e6a37a8cb1332e5b02594b530ff16e6d1fe6718b9d7be6f7bca"
                            .parse::<H256>()
                            .expect("hash to be valid")
                    )
                );
            }
            _ => panic!("unexpected error"),
        }
    }

    // not sure we want to fail, as the prover may work using an older version of STARK,
    //      it's still ok, the prover will possibly upgrade later.
    //  Also, in future we might extend the definition of metadata not to just reflect
    //  chronographic updates, but rather to support different schema formats depending
    //  on the chain key
    #[test]
    fn verifying_correct_stark_proof_when_program_metadata_config_is_different_should_error() {
        let proof_path = "../cairo/stone-verifier/proof_example_erc20.json";
        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = get_test_query();

        // note: the proof example above is all correct and generated by our Cairo program
        // however the STARK program metadata is configured for a different version of the
        // Cairo program thus rendering this input not to be authenticated
        let metadata = vec![(1, STARK_PROGRAM_V1_HASH)];

        let result = super::run_verifier(proof_example, query, metadata);

        assert!(result.is_err());

        let error = result.err().unwrap();

        match error {
            VerifierError::StarkProgramAuthError(e) => {
                assert_eq!(
                    e,
                    StarkProgramAuthError::AuthenticationFailure(STARK_PROGRAM_V3_HASH)
                );
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn verifying_stark_proof_with_incorrect_layout_segments_should_error() {
        let proof_path = "../cairo/stone-verifier/proof_example_erc20.json";
        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");
        let proof: StoneProofJson =
            serde_json::from_slice(&proof_example).expect("Unable to deserialize proof");

        let mut stone_proof = StoneProof::from(proof.clone());

        stone_proof
            .strip_off_annotations()
            .strip_off_prover_config()
            .strip_off_private_input();

        let mut query = get_test_query();
        // Number and size of segments not in accordance with proof
        query.layout_segments = vec![LayoutSegment { offset: 0, size: 0 }];

        let metadata = vec![(1, STARK_PROGRAM_V3_HASH)];

        let result = super::run_verifier(proof_example, query.clone(), metadata);

        assert!(result.is_err());

        let error = result.err().unwrap();

        match error {
            VerifierError::QueryValidationError(e) => {
                assert_eq!(
                    e,
                    super::QueryValidationError::QueryLayoutSegmentsError(
                        "Layout segments must have a non-zero byte size".to_string()
                    )
                );
            }
            _ => panic!("unexpected error"),
        }
    }

    fn cairo_verifier_output_from_proof_json(proof_path: &str) -> CairoVerifierOutput {
        let proof = std::fs::read(proof_path).expect("Proof example to be there");
        let proof: StoneProofJson = serde_json::from_slice(&proof).unwrap();
        let mut stone_proof = StoneProof::from(proof.clone());

        stone_proof
            .strip_off_annotations()
            .strip_off_prover_config()
            .strip_off_private_input();

        CairoVerifierOutput::try_from(stone_proof.proof())
            .map_err(|e| {
                error!("Failed to convert StoneProof to CairoVerifierOutput: {e:?}",);
                VerifierError::CairoVerifierOutputConversionError(e)
            })
            .unwrap()
    }

    // note: the proof example has changed, the proof_example.json file is now
    // in correspondence with the provided query and metadata (block 1, index 0, full data layout),
    // thus the proof is valid and should be verified successfully
    #[test]
    fn validate_query_against_proof_with_valid_proof_should_return_ok() {
        let query = get_test_query();
        let cairo_verifier_output = cairo_verifier_output_from_proof_json(
            "../cairo/stone-verifier/proof_example_erc20.json",
        );

        let result = validate_query_against_proof(query, &cairo_verifier_output);

        assert!(result.is_ok());
    }

    #[test]
    #[should_panic(expected = "QueryOutOfBounds")]
    fn validate_query_against_proof_when_query_index_is_larger_than_proof_index_should_error() {
        let mut query = get_test_query();
        query.index = 1;

        let cairo_verifier_output = cairo_verifier_output_from_proof_json(
            "../cairo/stone-verifier/proof_example_erc20.json",
        );

        validate_query_against_proof(query, &cairo_verifier_output).unwrap();
    }

    #[test]
    #[should_panic(expected = "QueryTransactionIdMismatch")]
    fn validate_query_against_proof_when_query_index_is_smaller_than_proof_index_should_error() {
        // Using alternate query matching proof_example_2nd_txn.json
        let query = Query {
            chain_id: 31337,
            height: 24, // different proof, block height is 24
            index: 0,   // proof has index of 1
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 418,
            }],
        };
        let cairo_verifier_output = cairo_verifier_output_from_proof_json(
            "../cairo/stone-verifier/proof_example_2nd_txn.json",
        );

        validate_query_against_proof(query, &cairo_verifier_output).unwrap();
    }

    #[test]
    #[should_panic(expected = "QueryOutOfBounds")]
    fn validate_query_against_proof_with_query_fields_mismatch_should_error() {
        let mut query = get_test_query();
        // Non-matching number and sizes of result segments
        query.layout_segments = vec![LayoutSegment {
            offset: 0,
            size: 1000,
        }];
        let mut cairo_verifier_output = cairo_verifier_output_from_proof_json(
            "../cairo/stone-verifier/proof_example_erc20.json",
        );
        // inject faulty state
        cairo_verifier_output.query_fields = felts_from_bytes(&NULL_ABI[..]);

        validate_query_against_proof(query, &cairo_verifier_output).unwrap();
    }

    fn check_result_segments_against_expected(actual_segments: Vec<ResultSegment>) {
        // Expected result segments correspond to the test ERC20 Transfer query in `get_test_query()`.
        // These have been validated against the fields of the original transaction, transaction
        // receipt, and query.
        let expected_segments: Vec<ResultSegment> = vec![
            ResultSegment {
                offset: 448,
                bytes: H256::from_slice(
                    &hex::decode(
                        "0000000000000000000000000000000000000000000000000000000000000001",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 192,
                bytes: H256::from_slice(
                    &hex::decode(
                        "000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 224,
                bytes: H256::from_slice(
                    &hex::decode(
                        "0000000000000000000000005fbdb2315678afecb367f032d93f642f64180aa3",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 800,
                bytes: H256::from_slice(
                    &hex::decode(
                        "0000000000000000000000005fbdb2315678afecb367f032d93f642f64180aa3",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 928,
                bytes: H256::from_slice(
                    &hex::decode(
                        "ddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 960,
                bytes: H256::from_slice(
                    &hex::decode(
                        "000000000000000000000000f39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 992,
                bytes: H256::from_slice(
                    &hex::decode(
                        "0000000000000000000000000000000000000000000000000000000000000001",
                    )
                    .expect("Decoding failed"),
                ),
            },
            ResultSegment {
                offset: 1056,
                bytes: H256::from_slice(
                    &hex::decode(
                        "0000000000000000000000000000000000000000000000000000000000000032",
                    )
                    .expect("Decoding failed"),
                ),
            },
        ];
        assert_eq!(expected_segments, actual_segments);
    }
}
