use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

const VERIFIER_COMMAND: &str = "cpu_air_verifier";

fn write_proof_to_temp_file(proof: &[u8]) -> std::io::Result<NamedTempFile> {
    let mut temp_file = NamedTempFile::new()?;
    temp_file.write_all(proof)?;
    Ok(temp_file)
}

pub fn run_verifier(proof: Vec<u8>) -> Result<String, String> {
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
    let output = Command::new(VERIFIER_COMMAND)
        .arg(format!("--in_file={}", temp_file_path))
        .stdout(Stdio::piped())
        .output()
        .map_err(|e| format!("Error executing verifier: {}", e.to_string()))?;

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
