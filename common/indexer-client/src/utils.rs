//! Utility functions for parsing GraphQL responses and building continuity proofs

use attestor_primitives::block::{Block, ContinuityProof};
use sp_core::H256;
use std::str::FromStr;

use crate::error::IndexerError;
use crate::types::{AttestationNode, AttestationNodeFull, AttestationWithProof, CheckpointNode};

/// Build ContinuityProof from blocks (used when continuity_proof field is needed)
pub fn build_continuity_proof_from_blocks(blocks: &[Block]) -> Option<ContinuityProof> {
    if blocks.is_empty() {
        return None;
    }
    let lower_endpoint_digest = blocks[0].prev_digest;
    let roots: Vec<H256> = blocks.iter().map(|b| b.root).collect();
    Some(ContinuityProof {
        lower_endpoint_digest,
        roots,
    })
}

/// Parse a single attestation node into AttestationWithProof
/// Works with both AttestationNode (from single queries) and AttestationNodeFull (from range queries)
pub fn parse_attestation_node(
    node: &AttestationNode,
    block_number: Option<u64>,
) -> Result<AttestationWithProof, IndexerError> {
    // Parse header_number from response or use provided one
    let block_number = if let Some(bn) = block_number {
        bn
    } else {
        node.header_number
            .as_ref()
            .ok_or_else(|| IndexerError::MissingField("header_number".to_string()))?
            .parse::<u64>()
            .map_err(|e| IndexerError::ParseInt {
                field: "header_number".to_string(),
                error: e.to_string(),
            })?
    };

    // Parse metadata
    let root = node
        .root
        .as_ref()
        .ok_or_else(|| IndexerError::MissingField("root".to_string()))?;
    let root = H256::from_str(root).map_err(|e| IndexerError::InvalidHex {
        field: "root".to_string(),
        error: e.to_string(),
    })?;

    let digest = node
        .digest
        .as_ref()
        .ok_or_else(|| IndexerError::MissingField("digest".to_string()))?;
    let digest = H256::from_str(digest).map_err(|e| IndexerError::InvalidHex {
        field: "digest".to_string(),
        error: e.to_string(),
    })?;

    let prev_digest = node
        .prev_digest
        .as_ref()
        .map(|s| {
            H256::from_str(s).map_err(|e| IndexerError::InvalidHex {
                field: "prev_digest".to_string(),
                error: e.to_string(),
            })
        })
        .transpose()?;

    // Store raw proof data as JSON - blocks will be extracted via extract_blocks() when needed
    let continuity_proof_data = node
        .continuity_proof
        .as_ref()
        .map(|proof_data| serde_json::to_value(proof_data).unwrap());

    // Build continuity_proof lazily from stored data if available
    let continuity_proof = if let Some(ref proof_data_json) = continuity_proof_data {
        // Use extract_blocks to parse and recompute blocks
        let temp_attestation = AttestationWithProof {
            block_number,
            root,
            digest,
            prev_digest,
            continuity_proof: None,
            continuity_proof_data: Some(proof_data_json.clone()),
        };
        temp_attestation
            .extract_blocks()
            .ok()
            .and_then(|blocks_opt| {
                blocks_opt.and_then(|blocks| build_continuity_proof_from_blocks(blocks.as_slice()))
            })
    } else {
        None
    };

    Ok(AttestationWithProof {
        block_number,
        root,
        digest,
        prev_digest,
        continuity_proof,
        continuity_proof_data,
    })
}

/// Parse AttestationNodeFull (from range queries) into AttestationWithProof
pub fn parse_attestation_node_full(
    node: &AttestationNodeFull,
) -> Result<AttestationWithProof, IndexerError> {
    let header_number = node
        .header_number
        .parse::<u64>()
        .map_err(|e| IndexerError::ParseInt {
            field: "header_number".to_string(),
            error: e.to_string(),
        })?;

    // Convert AttestationNodeFull to AttestationNode format for unified parsing
    let node_as_attestation = AttestationNode {
        header_number: Some(node.header_number.clone()),
        root: node.root.clone(),
        digest: node.digest.clone(),
        prev_digest: node.prev_digest.clone(),
        continuity_proof: node.continuity_proof.clone(),
    };

    parse_attestation_node(&node_as_attestation, Some(header_number))
}

/// Parse checkpoint node into AttestationCheckpoint
pub fn parse_checkpoint_node(
    node: &CheckpointNode,
) -> Result<attestor_primitives::AttestationCheckpoint, IndexerError> {
    let block_number = node
        .block_number
        .parse::<u64>()
        .map_err(|e| IndexerError::ParseInt {
            field: "block_number".to_string(),
            error: e.to_string(),
        })?;
    let digest = H256::from_str(&node.digest).map_err(|e| IndexerError::InvalidHex {
        field: "digest".to_string(),
        error: e.to_string(),
    })?;
    Ok(attestor_primitives::AttestationCheckpoint {
        block_number,
        digest,
    })
}
