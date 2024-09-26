use std::io::Write;
use std::process::{Command, Stdio};
use std::{env, fs, path::PathBuf};
use tempfile::NamedTempFile;

const VERIFIER_COMMAND: &str = "cairo/stone-verifier/cpu_air_verifier";

fn write_proof_to_temp_file(proof: &[u8]) -> std::io::Result<NamedTempFile> {
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(proof)?;
    Ok(temp_file)
}

pub fn run_verifier(proof: Vec<u8>) -> Result<String, String> {
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
        let project_root = crate::command::find_project_root()
            .ok_or("Could not find project root")
            .expect("project root to be found");
        let proof_path = project_root.join("cairo/stone-verifier/proof_example.json");

        let proof_example = std::fs::read(proof_path).expect("Proof example to be there");

        let result = super::run_verifier(proof_example);

        assert!(result.is_ok())
    }
}
