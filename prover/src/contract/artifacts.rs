use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json;
use std::path::Path;

use eth::evm::prover::GluwaPublicProverContract;

const ARTIFACT_STORAGE_FILE: &str = "artifacts/chain_deployment_artifacts.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainDeploymentArtifact {
    pub chain_id: u64,
    pub contract: GluwaPublicProverContract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactStorage {
    artifacts: Vec<ChainDeploymentArtifact>,
}

pub async fn has_artifact(chain_id: u64) -> Result<bool> {
    // Check if the file exists
    if !Path::new(ARTIFACT_STORAGE_FILE).exists() {
        return Ok(false);
    }

    // Read the existing file
    let data = tokio::fs::read_to_string(ARTIFACT_STORAGE_FILE).await?;
    let artifact_storage = serde_json::from_str::<ArtifactStorage>(&data)?;

    // Check if the artifact exists
    Ok(artifact_storage
        .artifacts
        .iter()
        .any(|artifact| artifact.chain_id == chain_id))
}

pub async fn get_deployment_artifact(chain_id: u64) -> Result<ChainDeploymentArtifact> {
    // Read the existing file
    let data = tokio::fs::read_to_string(ARTIFACT_STORAGE_FILE).await?;
    let artifact_storage = serde_json::from_str::<ArtifactStorage>(&data)?;

    // Find the artifact
    let artifact = artifact_storage
        .artifacts
        .iter()
        .find(|artifact| artifact.chain_id == chain_id)
        .ok_or_else(|| anyhow::anyhow!("Artifact not found"))?;

    Ok(artifact.clone())
}

pub async fn create_deployment_artifact(
    chain_id: u64,
    deployment: GluwaPublicProverContract,
) -> Result<()> {
    let artifact = ChainDeploymentArtifact {
        chain_id,
        contract: deployment,
    };

    // Check if the file exists
    let artifact_storage = if Path::new(ARTIFACT_STORAGE_FILE).exists() {
        // Read the existing file
        let data = tokio::fs::read_to_string(ARTIFACT_STORAGE_FILE).await?;
        serde_json::from_str::<ArtifactStorage>(&data)?
    } else {
        // If the file doesn't exist, create a new empty storage
        ArtifactStorage {
            artifacts: Vec::new(),
        }
    };

    // Create a mutable copy to add the new artifact
    let mut updated_artifact_storage = artifact_storage.clone();
    updated_artifact_storage.artifacts.push(artifact);

    // Serialize the updated storage and save it back to the file
    let serialized = serde_json::to_string_pretty(&updated_artifact_storage)?;
    let mut file = tokio::fs::File::create(ARTIFACT_STORAGE_FILE).await?;
    tokio::io::AsyncWriteExt::write_all(&mut file, serialized.as_bytes()).await?;

    Ok(())
}
