use anyhow::Result;
use pallet_prover_primitives::Query;
use prover_primitives::stark_program_auth::{
    StarkProgramAuth, StarkProgramMetadata, StarkProgramMetadataStorage,
};
use prover_primitives::types::{StoneProof, StoneProofJson};
use sp_runtime_interface::sp_wasm_interface::anyhow;
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::{env, fs, path::PathBuf};
use subxt::{OnlineClient, SubstrateConfig};
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

const VERIFIER_COMMAND: &str = "cairo/stone-verifier/cpu_air_verifier";

fn write_proof_to_temp_file(proof: &[u8]) -> std::io::Result<NamedTempFile> {
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(proof)?;
    Ok(temp_file)
}

#[subxt::subxt(runtime_metadata_path = "artifacts/metadata.scale")]
pub mod cc3 {}

pub struct Client {
    url: String,
    api: OnlineClient<SubstrateConfig>,
}

impl Client {
    pub async fn new(url: &str) -> Result<Self> {
        let api = if url.contains("ws") || url.contains("http") {
            OnlineClient::<SubstrateConfig>::from_insecure_url(&url).await?
        } else {
            OnlineClient::<SubstrateConfig>::from_url(&url).await?
        };

        Ok(Self {
            url: url.to_string(),
            api,
        })
    }

    pub(crate) async fn fetch_stark_program_metadata(&self) -> Result<StarkProgramMetadataStorage> {
        let last_version = self
            .api
            .storage()
            .at_latest()
            .await?
            .fetch(&cc3::storage().prover().last_version())
            .await?
            .unwrap_or(1);

        let mut map = HashMap::default();

        for i in 1..=last_version {
            let stark_program_metadata = self
                .api
                .storage()
                .at_latest()
                .await?
                .fetch(&cc3::storage().prover().stark_program_metadata(i))
                .await?
                .unwrap_or(0);

            map.insert(stark_program_metadata, StarkProgramMetadata { version: i });
        }

        Ok(StarkProgramMetadataStorage { map, last_version })
    }
}

pub fn default_stark_program_auth_hasher(bytes: &[u8]) -> u64 {
    use std::hash::DefaultHasher;
    use std::hash::Hash;
    use std::hash::Hasher;

    let mut hasher = DefaultHasher::new();
    bytes[..].hash(&mut hasher);

    hasher.finish()
}

pub fn run_verifier(proof: Vec<u8>, query: Query) -> Result<String, String> {
    let rt = Runtime::new().unwrap();

    let program_metadata_storage = rt.block_on(async {
        let cc_client = Client::new("ws://localhost:9944")
            .await
            .expect("Client to be created");

        cc_client
            .fetch_stark_program_metadata()
            .await
            .expect("Metadata to be fetched")
    });

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

    // Read the proof from the temporary file
    let proof_json = fs::read_to_string(temp_file_path)
        .map_err(|e| format!("Failed to read proof from temp file: {}", e))?;

    // Parse the proof JSON
    let proof: StoneProofJson = serde_json::from_str(&proof_json)
        .map_err(|e| format!("Failed to parse proof json: {}", e))?;

    let stone_proof = StoneProof::from(proof);

    // Authenticate the STARK program
    let metadata = StarkProgramAuth::authenticate(
        &stone_proof,
        &program_metadata_storage,
        default_stark_program_auth_hasher,
    )
    .map_err(|e| format!("Failed to authenticate STARK program: {e:?}"));

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

#[cfg(target_arch = "x86_64")]
pub mod tests {
    #[test]
    fn verify_works() {
        use pallet_prover_primitives::Query;

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

        let result = super::run_verifier(proof_example, query);

        println!("result: {:?}", result);

        assert!(result.is_ok());
    }
}
