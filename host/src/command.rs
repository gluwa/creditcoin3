use anyhow::Result;
use sp_core::H256;
use sp_runtime_interface::sp_wasm_interface::anyhow;
use std::{
    collections::HashMap,
    env, fs,
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};
use tempfile::NamedTempFile;

use pallet_prover_primitives::Query;
use prover_primitives::stark_program_auth::{
    StarkProgramAuth, StarkProgramAuthHash, StarkProgramMetadata, StarkProgramMetadataStorage,
};
use prover_primitives::types::{StoneProof, StoneProofJson};

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
) -> Result<String, String> {
    log::debug!("current dir: {:?}", env::current_dir().unwrap().as_os_str());

    // this code can be called from any directory within this project.
    // Here we find $PROJECT_ROOT/cairo/stone_verifier/cpu_air_verifier (where the stone verifier binary is located)
    // TODO: make building creditcoin3 also build the cpu_air_verifier and add it to the path so we can drop this locator
    let project_root = find_project_root().ok_or("Could not find project root")?;
    let verifier_path = project_root.join(VERIFIER_COMMAND);
    log::debug!("verifier bin path: {:?}", verifier_path);

    // Write proof to a temporary JSON file
    let temp_file = match write_proof_to_temp_file(&proof) {
        Ok(file) => file,
        Err(e) => return Err(format!("Failed to write proof to temp file: {}", e)),
    };

    // Ensure the temporary file is not deleted automatically
    let (_f, path) = temp_file
        .keep()
        .map_err(|e| format!("Failed to keep the temp file: {}", e))?;

    let temp_file_path = path.to_str().ok_or("Temp file not found".to_string())?;

    log::debug!("Created temp file with proof at: {}", temp_file_path);

    let proof: StoneProofJson =
        serde_json::from_slice(&proof).map_err(|e| format!("Failed to parse proof json: {}", e))?;

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
    let metadata = match StarkProgramAuth::authenticate(
        &stone_proof,
        &program_metadata_storage,
        blake2_256_stark_program_auth_hasher,
    ) {
        Ok(metadata) => metadata,
        Err(e) => return Err(format!("Failed to authenticate STARK program: {:?}", e)),
    };

    log::debug!("stark program authenticated with metadata: {:?}", metadata);

    // Execute the verifier command
    let output = Command::new(verifier_path)
        .arg(format!("--in_file={}", temp_file_path))
        .stdout(Stdio::piped())
        .output()
        .map_err(|e| format!("Error executing verifier: {e}"))?;

    // Remove the temporary file
    if let Err(e) = fs::remove_file(&path) {
        return Err(format!("Failed to remove temp file: {}", e));
    }

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(stderr)
    }
}

pub fn find_project_root() -> Option<PathBuf> {
    let mut current_dir = env::current_dir().ok()?;

    loop {
        if current_dir.join("SECURITY.md").exists() {
            return Some(current_dir);
        }

        // Move up to the parent directory
        if !current_dir.pop() {
            break;
        }
    }

    None
}

#[cfg(all(test, target_arch = "x86_64"))]
pub mod tests {
    use pallet_prover_primitives::{Query, STARK_PROGRAM_V1_HASH, STARK_PROGRAM_V2_HASH};

    #[test]
    fn verifying_authenticated_proof_should_return_ok() {
        let project_root = crate::command::find_project_root()
            .ok_or("Could not find project root")
            .expect("project root to be found");
        let proof_path = project_root.join("cairo/stone-verifier/proof_example.json");

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
        let project_root = crate::command::find_project_root()
            .ok_or("Could not find project root")
            .expect("project root to be found");

        // note: in this file the first 10 records in public_memory section have been altered
        // to 0x444 which should produce a different program hash and thus simulate
        // a STARK proof produced by an unauthorized/unauthenticated Cairo program
        // see StoneProof::program_bytes() and PublicInput::program_bytes() +
        // StarkProgramAuth::authenticate() for how the program hash is calculated!
        let proof_path = project_root.join("cairo/stone-verifier/bogus_public_memory_example.json");
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
        assert_eq!(
            result.err(),
            Some(
                "Failed to authenticate STARK program: AuthenticationFailure(0x2a9480cea28d8e6a37a8cb1332e5b02594b530ff16e6d1fe6718b9d7be6f7bca)"
                    .into()
            )
        );
    }

    // not sure we want to fail, as the prover may work using an older version of STARK,
    //      it's still ok, the prover will possibly upgrade later.
    //  Also, in future we might extend the definition of metadata not to just reflect
    //  chronographic updates, but rather to support different schema formats depending
    //  on the chain id
    #[test]
    fn verifying_correct_stark_proof_when_program_metadata_config_is_different_should_error() {
        let project_root = crate::command::find_project_root()
            .ok_or("Could not find project root")
            .expect("project root to be found");
        let proof_path = project_root.join("cairo/stone-verifier/proof_example.json");
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
        assert_eq!(
            result.err(),
            Some(format!(
                "Failed to authenticate STARK program: AuthenticationFailure({:?})",
                STARK_PROGRAM_V2_HASH
            ))
        );
    }
}
