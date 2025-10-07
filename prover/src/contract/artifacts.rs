use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json;
use sp_core::H256;
use std::path::{Path, PathBuf};

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

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    storage_path: PathBuf,
}

impl ArtifactStore {
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self {
            storage_path: path.into(),
        }
    }

    pub async fn has_artifact(&self, chain_id: u64) -> Result<bool> {
        if !Path::new(&self.storage_path).exists() {
            return Ok(false);
        }

        let data = tokio::fs::read_to_string(&self.storage_path).await?;
        let artifact_storage = serde_json::from_str::<ArtifactStorage>(&data)?;

        Ok(artifact_storage
            .artifacts
            .iter()
            .any(|artifact| artifact.chain_id == chain_id))
    }

    pub async fn get_latest_deployment_artifact_for(
        &self,
        chain_id: u64,
    ) -> Result<ChainDeploymentArtifact> {
        let data = tokio::fs::read_to_string(&self.storage_path).await?;
        let artifact_storage = serde_json::from_str::<ArtifactStorage>(&data)?;

        let artifact = artifact_storage
            .artifacts
            .iter()
            .filter(|artifact| artifact.chain_id == chain_id)
            .max_by(|a, b| a.created_at.cmp(&b.created_at))
            .ok_or_else(|| anyhow::anyhow!("Artifact not found"))?;

        Ok(artifact.clone())
    }

    pub async fn create_deployment_artifact(
        &self,
        chain_id: u64,
        deployment: GluwaPublicProverContract,
        bytecode_hash: H256,
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

        if let Some(parent_dir) = Path::new(&self.storage_path).parent() {
            tokio::fs::create_dir_all(parent_dir).await?;
        }

        let mut artifact_storage = if Path::new(&self.storage_path).exists() {
            let data = tokio::fs::read_to_string(&self.storage_path).await?;
            serde_json::from_str::<ArtifactStorage>(&data)?
        } else {
            ArtifactStorage {
                artifacts: Vec::new(),
            }
        };

        artifact_storage.artifacts.push(artifact.clone());

        let serialized = serde_json::to_string_pretty(&artifact_storage)?;
        let mut file = tokio::fs::File::create(&self.storage_path).await?;
        tokio::io::AsyncWriteExt::write_all(&mut file, serialized.as_bytes()).await?;

        Ok(artifact)
    }
}
