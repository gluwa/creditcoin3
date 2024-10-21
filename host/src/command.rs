use anyhow::Result;
use sp_core::H256;
use sp_runtime_interface::sp_wasm_interface::anyhow;
use std::{
    collections::HashMap,
    env, fs,
    io::Write,
    process::{Command, Stdio},
};
use tempfile::NamedTempFile;

use pallet_prover_primitives::Query;
use prover_primitives::stark_program_auth::{
    StarkProgramAuth, StarkProgramAuthHash, StarkProgramMetadata, StarkProgramMetadataStorage,
};
use prover_primitives::types::{StoneProof, StoneProofJson};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum VerifierError {
    #[error("Failed to find project root")]
    ProjectRootNotFound,

    #[error("Failed to write proof to temp file: {0}")]
    TempFileWriteError(std::io::Error),

    #[error("Failed to keep the temp file: {0}")]
    TempFileKeepError(tempfile::PersistError),

    #[error("Temp file not found")]
    TempFileNotFound,

    #[error("Failed to parse proof JSON: {0}")]
    ProofParseError(serde_json::Error),

    #[error("Failed to authenticate STARK program: {0}")]
    StarkProgramAuthError(String),

    #[error("Error executing verifier: {0}")]
    VerifierExecutionError(std::io::Error),

    #[error("Verifier process failed with stderr: {0}")]
    VerifierProcessError(String),

    #[error("Failed to remove temp file: {0}")]
    TempFileRemoveError(std::io::Error),
}

const VERIFIER_COMMAND: &str = "cairo/stone-verifier/cpu_air_verifier";

fn write_proof_to_temp_file(proof: &[u8]) -> std::io::Result<NamedTempFile> {
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(proof)?;
    Ok(temp_file)
}

fn blake2_256_stark_program_auth_hasher(bytes: &[u8]) -> StarkProgramAuthHash {
    H256::from(sp_io::hashing::blake2_256(bytes))
}

pub fn run_verifier(
    proof: Vec<u8>,
    _query: Query,
    metadata: Vec<(u8, StarkProgramAuthHash)>,
) -> Result<String, VerifierError> {
    log::debug!("current dir: {:?}", env::current_dir().unwrap().as_os_str());

    // Write proof to a temporary JSON file
    let temp_file = write_proof_to_temp_file(&proof).map_err(VerifierError::TempFileWriteError)?;

    // Ensure the temporary file is not deleted automatically
    let (_f, path) = temp_file
        .keep()
        .map_err(|e| VerifierError::TempFileKeepError(e))?;

    let temp_file_path = path.to_str().ok_or(VerifierError::TempFileNotFound)?;

    log::debug!("Created temp file with proof at: {}", temp_file_path);

    let proof: StoneProofJson =
        serde_json::from_slice(&proof).map_err(VerifierError::ProofParseError)?;

    let stone_proof = StoneProof::from(proof);

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
    )
    .map_err(|e| VerifierError::StarkProgramAuthError(format!("{:?}", e)))?;

    log::debug!("stark program authenticated with metadata: {:?}", metadata);

    // Execute the verifier command
    // WARNING: binary must be in $PATH and/or $PATH must be configured accordingly
    let output = Command::new("cpu_air_verifier")
        .arg(format!("--in_file={}", temp_file_path))
        .stdout(Stdio::piped())
        .output()
        .map_err(VerifierError::VerifierExecutionError)?;

    fs::remove_file(&path).map_err(VerifierError::TempFileRemoveError)?;

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
    use pallet_prover_primitives::{Query, STARK_PROGRAM_V1_HASH, STARK_PROGRAM_V2_HASH};

    #[test]
    fn verifying_authenticated_proof_should_return_ok() {
        let proof_path = "../cairo/stone-verifier/proof_example.json";

        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let query = Query {
            chain_id: 31337,
            height: 1,
            index: 1,
            layout_segments: vec![],
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
        // assert_eq!(
        //     result.err(),
        //     Some(VerifierError::StarkProgramAuthError("AuthenticationFailure".to_string())));
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
            index: 1,
            layout_segments: vec![],
        };

        // note: the proof example above is all correct and generated by our Cairo program
        // however the STARK program metadata is configured for a different version of the
        // Cairo program thus rendering this input not to be authenticated
        let metadata = vec![(1, STARK_PROGRAM_V1_HASH)];

        let result = super::run_verifier(proof_example, query, metadata);

        assert!(result.is_err());
        // assert_eq!(
        //     result.err(),
        //     Some(VerifierError::StarkProgramAuthError("AuthenticationFailure".to_string()))
        // );
    }
}
