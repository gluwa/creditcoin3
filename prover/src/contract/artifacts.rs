use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json;
use sp_core::H256;
use std::path::Path;

use eth::evm::prover::GluwaPublicProverContract;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainDeploymentArtifact {
    pub chain_id: u64,
    pub contract: GluwaPublicProverContract,
    #[serde(default)]
    pub bytecode_hash: Option<H256>,
    #[serde(default)]
    pub created_at: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactStorage {
    artifacts: Vec<ChainDeploymentArtifact>,
}

pub async fn has_artifact(chain_id: u64, path: &str) -> Result<bool> {
    // Check if the file exists
    if !Path::new(path).exists() {
        return Ok(false);
    }

    // Read the existing file
    let data = tokio::fs::read_to_string(path).await?;
    let artifact_storage = serde_json::from_str::<ArtifactStorage>(&data)?;

    // Check if the artifact exists
    Ok(artifact_storage
        .artifacts
        .iter()
        .any(|artifact| artifact.chain_id == chain_id))
}

pub async fn get_latest_deployment_artifact_for(
    chain_id: u64,
    path: &str,
) -> Result<ChainDeploymentArtifact> {
    // Read the existing file
    let data = tokio::fs::read_to_string(path).await?;
    let artifact_storage = serde_json::from_str::<ArtifactStorage>(&data)?;

    // Find the artifact
    let artifact = artifact_storage
        .artifacts
        .iter()
        .filter(|artifact| artifact.chain_id == chain_id)
        .max_by(|a, b| a.created_at.cmp(&b.created_at))
        .ok_or_else(|| anyhow::anyhow!("Artifact not found"))?;

    Ok(artifact.clone())
}

pub async fn create_deployment_artifact(
    chain_id: u64,
    deployment: GluwaPublicProverContract,
    bytecode_hash: H256,
    path: &str,
) -> Result<ChainDeploymentArtifact> {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();

    let artifact = ChainDeploymentArtifact {
        chain_id,
        contract: deployment,
        bytecode_hash: Some(bytecode_hash),
        created_at,
    };

    // Ensure the parent directory exists
    if let Some(parent_dir) = Path::new(path).parent() {
        tokio::fs::create_dir_all(parent_dir).await?;
    }

    // Check if the file exists
    let mut artifact_storage = if Path::new(path).exists() {
        // Read the existing file
        let data = tokio::fs::read_to_string(path).await?;
        serde_json::from_str::<ArtifactStorage>(&data)?
    } else {
        // If the file doesn't exist, create a new empty storage
        ArtifactStorage {
            artifacts: Vec::new(),
        }
    };

    // Add the new artifact
    artifact_storage.artifacts.push(artifact.clone());

    // Serialize the updated storage and save it back to the file
    let serialized = serde_json::to_string_pretty(&artifact_storage)?;
    let mut file = tokio::fs::File::create(path).await?;
    tokio::io::AsyncWriteExt::write_all(&mut file, serialized.as_bytes()).await?;

    Ok(artifact)
}
