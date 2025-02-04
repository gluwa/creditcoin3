use anyhow::Result;
use core::cmp::Ordering::*;
use sp_core::H256;
use sp_runtime_interface::sp_wasm_interface::anyhow;
use std::{
    collections::HashMap,
    env, fs,
    io::Write,
    ops::Range,
    process::{Command, Stdio},
};
use tempfile::{NamedTempFile, PersistError};
use thiserror::Error;

use pallet_prover_primitives::Query;
use prover_primitives::claim::ClaimValidationError;
use prover_primitives::claim::ClaimValidationError::*;
use prover_primitives::stark_program_auth::{
    StarkProgramAuth, StarkProgramAuthError, StarkProgramAuthHash, StarkProgramMetadata,
    StarkProgramMetadataStorage,
};
use prover_primitives::types::{CairoVerifierOutput, StoneProof, StoneProofJson};
use utils::pedersen_hash::pedersen_array;
use utils::{utils::felts_from_bytes, Felt};

#[derive(Error, Debug)]
pub enum VerifierError {
    #[error("Io error")]
    IoError(#[from] std::io::Error),

    #[error("Failed to persist temp file")]
    TempFilePersistError(#[from] PersistError),

    #[error("Failed to keep the temp file")]
    TempFileKeepError,

    #[error("Temp file not found")]
    TempFileNotFound,

    #[error("Failed to parse proof JSON")]
    ProofParseError(#[from] serde_json::Error),

    #[error("Failed to convert StoneProof to CairoVerifierOutput: {0}")]
    CairoVerifierOutputConversionError(String),

    #[error("Failed to authenticate STARK program: {0}")]
    StarkProgramAuthError(#[from] StarkProgramAuthError),

    #[error("Error executing verifier")]
    VerifierExecutionError,

    #[error("Verifier process failed with stderr: {0}")]
    VerifierProcessError(String),

    #[error("Claim validation error: {0}")]
    QueryValidationError(#[from] ClaimValidationError),

    #[error("Failed to remove temp file")]
    TempFileRemoveError,
}

impl VerifierError {
    pub fn status_code(&self) -> u8 {
        match self {
            VerifierError::IoError(e) => {
                log::error!("error writing to temp file: {:?}", e);
                1
            }
            VerifierError::TempFilePersistError(e) => {
                log::error!("error persisting temp file: {:?}", e);
                2
            }
            VerifierError::TempFileKeepError => {
                log::error!("error keeping temp file");
                3
            }
            VerifierError::TempFileNotFound => {
                log::error!("temp file not found");
                4
            }
            VerifierError::TempFileRemoveError => {
                log::error!("io error");
                5
            }
            VerifierError::ProofParseError(e) => {
                log::error!("error parsing the proof: {:?}", e);
                6
            }
            VerifierError::CairoVerifierOutputConversionError(e) => {
                log::error!(
                    "error converting StoneProof to CairoVerifierOutput: {:?}",
                    e
                );
                7
            }
            VerifierError::StarkProgramAuthError(e) => {
                log::error!("stark program authentication error: {:?}", e);
                8
            }
            VerifierError::VerifierExecutionError => {
                log::error!("error running verifier");
                9
            }
            VerifierError::VerifierProcessError(e) => {
                log::error!("verifier was not able to verify the proof: {:?}", e);
                10
            }
            VerifierError::QueryValidationError(e) => match e {
                QueryIdNotValidated(expected, found) => {
                    log::error!(
                        "query id not validated, expected {:?}, found {:?}",
                        expected,
                        found
                    );
                    11
                }
                QueryOutOfBounds(index) => {
                    log::error!("claim out of bounds at index: {}", index);
                    12
                }
                QueryOffsetsMismatch(expected, found) => {
                    log::error!("query offsets mismatch, {:?}, {:?}", expected, found);
                    13
                }
                FieldNotValidated(range, found, expected) => {
                    log::error!(
                        "field at range {:?} not validated, expected {:?}, found {:?}",
                        range,
                        expected,
                        found
                    );
                    14
                }
                FieldInner(e) => {
                    log::error!("field inner error: {:?}", e);
                    15
                }
                ProofOutputTruncated => {
                    log::error!("proof output truncated");
                    16
                }
            },
        }
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
    mut query: Query,
    cairo_verifier_output: &CairoVerifierOutput,
) -> Result<(), ClaimValidationError> {
    match query.index.cmp(&cairo_verifier_output.claim_index) {
        Greater => Err(QueryOutOfBounds(cairo_verifier_output.claim_index)),

        Equal => {
            if felts_from_bytes(&rlp::NULL_RLP[..]) == cairo_verifier_output.claim_fields {
                Err(QueryOutOfBounds(cairo_verifier_output.claim_index))
            } else {
                query.transform_to_felt_offsets();
                let local_offset_hash = match hash_layout_segments(&query) {
                    Ok(hash) => hash,
                    Err(e) => {
                        log::error!("Failed to hash layout segments: {:?}", e);
                        return Err(QueryIdNotValidated(
                            query.index,
                            cairo_verifier_output.claim_index,
                        ));
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

        Less => Err(QueryIdNotValidated(
            query.index,
            cairo_verifier_output.claim_index,
        )),
    }
}

pub fn hash_layout_segments(query: &Query) -> Result<Felt, &'static str> {
    let felt_ranges: Vec<Range<u64>> = query
        .layout_segments
        .iter()
        .map(|layout| {
            layout
                .offset
                .checked_add(layout.size)
                .map(|end| Range {
                    start: layout.offset,
                    end,
                })
                .ok_or("Overflow detected in layout segment")
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut felts_offsets = Vec::new();
    for r in felt_ranges {
        let range_len = (r.end - r.start) as usize;
        felts_offsets
            .try_reserve(range_len)
            .map_err(|_| "layout range is too large")?;
        felts_offsets.extend((r.start..r.end).map(Into::<Felt>::into));
    }

    Ok(pedersen_array(&felts_offsets))
}

pub fn run_verifier(
    proof: Vec<u8>,
    query: Query,
    metadata: Vec<(u8, StarkProgramAuthHash)>,
) -> Result<String, VerifierError> {
    log::debug!("current dir: {:?}", env::current_dir()?.as_os_str());

    // Write proof to a temporary JSON file
    let temp_file_path = write_proof_to_temp_file(&proof)?;

    log::debug!("Created temp file with proof at: {}", temp_file_path);

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
            log::error!(
                "Failed to convert StoneProof to CairoVerifierOutput: {:?}",
                e
            );
            VerifierError::CairoVerifierOutputConversionError(e)
        })?;

    match validate_query_against_proof(query, &cairo_verifier_output) {
        Ok(_) => log::debug!("Query validated successfully"),
        Err(e) => return Err(VerifierError::QueryValidationError(e)),
    }

    log::debug!("stark program authenticated with metadata: {:?}", metadata);

    // Execute the verifier command
    // WARNING: binary must be in $PATH and/or $PATH must be configured accordingly
    let output = Command::new("cpu_air_verifier")
        .arg(format!("--in_file={}", temp_file_path))
        .stdout(Stdio::piped())
        .output()?;

    fs::remove_file(&temp_file_path)?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(VerifierError::VerifierProcessError(stderr))
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
pub mod tests {
    use crate::command::{
        felts_from_bytes, hash_layout_segments, validate_query_against_proof, VerifierError,
    };
    use pallet_prover_primitives::{
        LayoutSegment, Query, STARK_PROGRAM_V1_HASH, STARK_PROGRAM_V2_HASH,
    };
    use prover_primitives::{
        stark_program_auth::StarkProgramAuthError,
        types::{CairoVerifierOutput, StoneProof, StoneProofJson},
    };
    use sp_core::H256;

    // note: the proof example has changed, the proof_example.json file is now
    // in correspondence with the provided query and metadata (block 1, index 0, full data layout),
    // thus the proof is valid and should be verified successfully
    #[test]
    fn verifying_authenticated_proof_should_return_ok() {
        let proof_path = "../cairo/stone-verifier/proof_example.json";

        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 0,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 418,
            }],
        };

        let metadata = vec![(1, STARK_PROGRAM_V2_HASH)];

        let result = super::run_verifier(proof_example, query, metadata);

        assert!(result.is_ok());
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

        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 1,
            layout_segments: vec![],
        };

        let metadata = vec![(1, STARK_PROGRAM_V2_HASH)];

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
        let proof_path = "../cairo/stone-verifier/proof_example.json";
        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 0,
            layout_segments: vec![],
        };

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
                    StarkProgramAuthError::AuthenticationFailure(STARK_PROGRAM_V2_HASH)
                );
            }
            _ => panic!("unexpected error"),
        }
    }

    #[test]
    fn verifying_stark_proof_with_incorrect_layout_segments_should_error() {
        let proof_path = "../cairo/stone-verifier/proof_example.json";
        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");
        let proof: StoneProofJson =
            serde_json::from_slice(&proof_example).expect("Unable to deserialize proof");

        let mut stone_proof = StoneProof::from(proof.clone());

        stone_proof
            .strip_off_annotations()
            .strip_off_prover_config()
            .strip_off_private_input();

        let cairo_verifier_output = CairoVerifierOutput::try_from(stone_proof.proof())
            .map_err(|e| {
                log::error!(
                    "Failed to convert StoneProof to CairoVerifierOutput: {:?}",
                    e
                );
                VerifierError::CairoVerifierOutputConversionError(e)
            })
            .expect("Unable to convert StoneProof to CairoVerifierOutput");

        let mut query = Query {
            chain_id: 31337,
            height: 1,
            index: 0,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 500, // size not in accordance with the proof
            }],
        };

        let metadata = vec![(1, STARK_PROGRAM_V2_HASH)];

        let result = super::run_verifier(proof_example, query.clone(), metadata);

        assert!(result.is_err());

        let error = result.err().unwrap();

        query.transform_to_felt_offsets();
        let local_offset_hash = hash_layout_segments(&query).unwrap();

        match error {
            VerifierError::QueryValidationError(e) => {
                assert_eq!(
                    e,
                    super::ClaimValidationError::QueryOffsetsMismatch(
                        cairo_verifier_output.query_hash,
                        local_offset_hash
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
                log::error!(
                    "Failed to convert StoneProof to CairoVerifierOutput: {:?}",
                    e
                );
                VerifierError::CairoVerifierOutputConversionError(e)
            })
            .unwrap()
    }

    // note: the proof example has changed, the proof_example.json file is now
    // in correspondence with the provided query and metadata (block 1, index 0, full data layout),
    // thus the proof is valid and should be verified successfully
    #[test]
    fn validate_query_against_proof_with_valid_proof_should_return_ok() {
        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 0,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 418,
            }],
        };
        let cairo_verifier_output =
            cairo_verifier_output_from_proof_json("../cairo/stone-verifier/proof_example.json");

        let result = validate_query_against_proof(query, &cairo_verifier_output);

        assert!(result.is_ok());
    }

    #[test]
    #[should_panic(expected = "QueryOutOfBounds")]
    fn validate_query_against_proof_when_query_index_is_larger_than_proof_index_should_error() {
        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 1, // proof has index of 0
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 418,
            }],
        };
        let cairo_verifier_output =
            cairo_verifier_output_from_proof_json("../cairo/stone-verifier/proof_example.json");

        validate_query_against_proof(query, &cairo_verifier_output).unwrap();
    }

    #[test]
    #[should_panic(expected = "QueryIdNotValidated")]
    fn validate_query_against_proof_when_query_index_is_smaller_than_proof_index_should_error() {
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
    fn validate_query_against_proof_with_claim_fields_mismatch_should_error() {
        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 0,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 418,
            }],
        };
        let mut cairo_verifier_output =
            cairo_verifier_output_from_proof_json("../cairo/stone-verifier/proof_example.json");
        // inject faulty state
        cairo_verifier_output.claim_fields = felts_from_bytes(&rlp::NULL_RLP[..]);

        validate_query_against_proof(query, &cairo_verifier_output).unwrap();
    }

    #[test]
    #[should_panic(expected = "QueryIdNotValidated")]
    fn validate_query_against_proof_should_error_when_layout_segments_cannot_be_hashed() {
        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 0,
            layout_segments: vec![LayoutSegment {
                // values too large, will cause hash_layout_segments() to error internally
                offset: u64::MAX,
                size: u64::MAX,
            }],
        };
        let cairo_verifier_output =
            cairo_verifier_output_from_proof_json("../cairo/stone-verifier/proof_example.json");

        validate_query_against_proof(query, &cairo_verifier_output).unwrap();
    }
}
