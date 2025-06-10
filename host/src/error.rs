use log::error;
use tempfile::PersistError;
use thiserror::Error;

use prover_primitives::{
    claim::ClaimValidationError::{self, *},
    stark_program_auth::StarkProgramAuthError,
};

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
                error!("error writing to temp file: {:?}", e);
                1
            }
            VerifierError::TempFilePersistError(e) => {
                error!("error persisting temp file: {:?}", e);
                2
            }
            VerifierError::TempFileKeepError => {
                error!("error keeping temp file");
                3
            }
            VerifierError::TempFileNotFound => {
                error!("temp file not found");
                4
            }
            VerifierError::TempFileRemoveError => {
                error!("io error");
                5
            }
            VerifierError::ProofParseError(e) => {
                error!("error parsing the proof: {:?}", e);
                6
            }
            VerifierError::CairoVerifierOutputConversionError(e) => {
                error!(
                    "error converting StoneProof to CairoVerifierOutput: {:?}",
                    e
                );
                7
            }
            VerifierError::StarkProgramAuthError(e) => {
                error!("stark program authentication error: {:?}", e);
                8
            }
            VerifierError::VerifierExecutionError => {
                error!("error running verifier");
                9
            }
            VerifierError::VerifierProcessError(e) => {
                error!("verifier was not able to verify the proof: {:?}", e);
                10
            }
            VerifierError::QueryValidationError(e) => match e {
                FailedToHashLayoutsegments(msg) => {
                    error!("failed to hash layout segments: {}", msg);
                    11
                }
                QueryOutOfBounds(index) => {
                    error!("claim out of bounds at index: {}", index);
                    12
                }
                QueryOffsetsMismatch(expected, found) => {
                    error!("query offsets mismatch, {:?}, {:?}", expected, found);
                    13
                }
                FieldNotValidated(range, found, expected) => {
                    error!(
                        "field at range {:?} not validated, expected {:?}, found {:?}",
                        range, expected, found
                    );
                    14
                }
                FieldInner(e) => {
                    error!("field inner error: {:?}", e);
                    15
                }
                ProofOutputTruncated => {
                    error!("proof output truncated");
                    16
                }
                QueryLayoutSegmentsError(msg) => {
                    error!("query layout segments error: {}", msg);
                    17
                }
                QueryTransactionIdMismatch(found, expected) => {
                    error!(
                        "query transaction id mismatch, found: {}, expected: {}",
                        found, expected
                    );
                    18
                }
            },
        }
    }
}
