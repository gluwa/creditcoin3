//! GraphQL request/response types for the indexer API

use serde::{Deserialize, Serialize};

/// Query variables for attestation queries (used in tests)
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryVariables {
    pub chain_key: String,
    pub header_number: String,
}

/// GraphQL error structure
#[derive(Deserialize)]
pub struct GraphQLError {
    pub message: String,
}

/// Response data wrapper
#[derive(Deserialize)]
pub struct ResponseData {
    pub attestations: AttestationsConnection,
}

/// Attestations connection (list of attestations)
#[derive(Deserialize)]
pub struct AttestationsConnection {
    pub nodes: Vec<AttestationNode>,
}

/// Attestation node from GraphQL response.
/// Includes root, digest, prevDigest, and continuityProof fields.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AttestationNode {
    #[serde(default)]
    pub header_number: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub prev_digest: Option<String>,
    pub continuity_proof: Option<ContinuityProofData>,
}

/// The continuityProof JSON blob from the indexer.
/// Contains an array of blocks that form the continuity proof.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct ContinuityProofData {
    pub blocks: Vec<ContinuityBlockData>,
}

/// Individual block within a continuity proof.
/// Field names match the actual JSON response from the indexer.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ContinuityBlockData {
    pub block_number: u64,
    pub root: String,
    pub prev_digest: Option<String>,
    #[allow(dead_code)] // We recompute digests instead of using indexer values
    pub digest: String,
}

/// Checkpoints connection (list of checkpoints)
#[derive(Deserialize)]
pub struct CheckpointsConnection {
    pub nodes: Vec<CheckpointNode>,
}

/// Checkpoint node from GraphQL response
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointNode {
    pub block_number: String,
    pub digest: String,
}

/// Checkpoint response data wrapper
#[derive(Deserialize)]
pub struct CheckpointResponseData {
    pub checkpoints: CheckpointsConnection,
}

// ============================================================================
// Query Variable Structs
// ============================================================================

/// Query variables for range queries (attestations or checkpoints)
#[derive(Serialize)]
pub struct RangeQueryVariables {
    #[serde(rename = "chainKey")]
    pub chain_key: String,
    #[serde(rename = "minBlock")]
    pub min_block: String,
    #[serde(rename = "maxBlock")]
    pub max_block: String,
    /// Query height for checkpoint queries (used to separate before/after checkpoints)
    #[serde(rename = "queryHeight", skip_serializing_if = "Option::is_none")]
    pub query_height: Option<String>,
}

/// Query variables for checkpoint queries
#[derive(Serialize)]
pub struct CheckpointQueryVariables {
    #[serde(rename = "chainKey")]
    pub chain_key: String,
}

/// Query variables for checkpoint by block queries
#[derive(Serialize)]
pub struct CheckpointByBlockVariables {
    #[serde(rename = "chainKey")]
    pub chain_key: String,
    #[serde(rename = "blockNumber")]
    pub block_number: String,
}

/// Generic GraphQL query wrapper
#[derive(Serialize)]
pub struct GraphQLQueryWrapper<T> {
    pub query: &'static str,
    pub variables: T,
}

// ============================================================================
// Response Structs
// ============================================================================

/// Generic GraphQL response wrapper
#[derive(Deserialize)]
pub struct GraphQLResponseWrapper<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQLError>>,
}

/// Attestation node for range queries (includes all fields)
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AttestationNodeFull {
    pub header_number: String,
    pub root: Option<String>,
    pub digest: Option<String>,
    pub prev_digest: Option<String>,
    pub continuity_proof: Option<ContinuityProofData>,
}

/// Attestation connection for range queries
#[derive(Deserialize)]
pub struct AttestationsConnectionFull {
    pub nodes: Vec<AttestationNodeFull>,
}

/// Response data for range attestation queries
#[derive(Deserialize)]
pub struct AttestationsRangeResponseData {
    pub attestations: AttestationsConnectionFull,
}

/// Checkpoints in range response data
///
/// Note: This structure separates checkpoints before and after the query because
/// the GraphQL query uses different ordering for each (DESC for before, ASC for after).
/// The checkpoints are immediately combined into a single vector when used.
#[derive(Deserialize)]
pub struct CheckpointsInRangeData {
    #[serde(rename = "checkpointsBefore")]
    pub checkpoints_before: CheckpointNodes,
    #[serde(rename = "checkpointsAfter")]
    pub checkpoints_after: CheckpointNodes,
}

/// Checkpoint nodes wrapper
#[derive(Deserialize)]
pub struct CheckpointNodes {
    pub nodes: Vec<CheckpointNode>,
}

/// Unified attestation data structure containing both metadata and proof.
/// This avoids duplicate GraphQL queries when both are needed.
#[derive(Debug, Clone)]
pub struct AttestationWithProof {
    /// Attestation root
    pub root: sp_core::H256,
    /// Attestation digest
    pub digest: sp_core::H256,
    /// Previous digest (optional)
    pub prev_digest: Option<sp_core::H256>,
    /// Block number for this attestation
    pub block_number: u64,
    /// Continuity proof (optional, may not be available for all attestations)
    pub continuity_proof: Option<attestor_primitives::block::ContinuityProof>,
    /// Raw continuity proof data with block numbers (for extracting blocks)
    /// This is an opaque JSON value that indexer implementations can use internally
    pub continuity_proof_data: Option<serde_json::Value>,
}

impl AttestationWithProof {
    /// Create from SignedAttestation (for CC3 chain data)
    pub fn from_signed_attestation<AccountId>(
        attestation: &attestor_primitives::SignedAttestation<sp_core::H256, AccountId>,
    ) -> Self {
        Self {
            block_number: attestation.attestation.header_number,
            root: attestation.attestation.root,
            digest: attestation.digest(),
            prev_digest: attestation.prev_digest(),
            continuity_proof: None,
            continuity_proof_data: None,
        }
    }

    /// Create from AttestationCheckpoint (for checkpoint data)
    /// Note: Checkpoints don't have root, so we use a default value
    pub fn from_checkpoint(checkpoint: &attestor_primitives::AttestationCheckpoint) -> Self {
        Self {
            block_number: checkpoint.block_number,
            root: sp_core::H256::default(), // Checkpoints don't have root
            digest: checkpoint.digest,
            prev_digest: None,
            continuity_proof: None,
            continuity_proof_data: None,
        }
    }

    /// Extract continuity blocks from the proof data.
    /// This parses the JSON proof data into Block structures.
    ///
    /// # Returns
    ///
    /// - `Ok(Some(blocks))` - Continuity blocks parsed successfully
    /// - `Ok(None)` - No continuity proof data available
    /// - `Err(_)` - Parsing error
    pub fn extract_blocks(&self) -> anyhow::Result<Option<Vec<attestor_primitives::block::Block>>> {
        use std::str::FromStr;

        let Some(ref proof_data_json) = self.continuity_proof_data else {
            return Ok(None);
        };

        // Deserialize JSON to ContinuityProofData structure
        // Use camelCase to match the GraphQL response format
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ContinuityBlockData {
            block_number: u64,
            root: String,
            prev_digest: Option<String>,
        }

        #[derive(serde::Deserialize)]
        struct ContinuityProofData {
            blocks: Vec<ContinuityBlockData>,
        }

        let proof_data: ContinuityProofData = serde_json::from_value(proof_data_json.clone())?;

        // Parse and recompute blocks
        let mut blocks = Vec::new();
        let mut last_digest = sp_core::H256::default();

        for (idx, b) in proof_data.blocks.iter().enumerate() {
            let root = sp_core::H256::from_str(&b.root)
                .map_err(|e| anyhow::anyhow!("Invalid root hex: {e}"))?;

            let prev_digest = if let Some(ref stored_prev_digest) = b.prev_digest {
                sp_core::H256::from_str(stored_prev_digest)
                    .map_err(|e| anyhow::anyhow!("Invalid prev_digest hex: {e}"))?
            } else if idx == 0 {
                return Err(anyhow::anyhow!("Missing prev_digest for first block"));
            } else {
                last_digest
            };

            let digest = attestor_primitives::block::Block::hash_payload(
                &b.block_number,
                &root,
                &prev_digest,
            );

            blocks.push(attestor_primitives::block::Block {
                block_number: b.block_number,
                root,
                prev_digest,
                digest,
            });

            last_digest = digest;
        }

        Ok(Some(blocks))
    }
}
