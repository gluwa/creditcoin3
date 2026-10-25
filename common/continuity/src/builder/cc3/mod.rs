//! CC3 chain-based continuity proof building
//!
//! This module contains functions for building continuity proofs by querying
//! the CC3 chain directly. These are slower than using the indexer but work
//! when no indexer is available.

use super::super::ContinuityBuilder;
use crate::builder::proof_builder::ContinuityResult;
use crate::errors::ContinuityError;
use attestor_primitives::block::Block;
use indexer_client::AttestationWithProof;
use sp_core::H256;
use tracing::info;

impl ContinuityBuilder {
    /// Helper to build from source chain (fallback when indexer unavailable).
    /// Returns (blocks, lower_endpoint_digest) tuple matching the indexer path return type.
    /// The lower_endpoint_digest is extracted from the first block's prev_digest, which should
    /// match the digest of the attestation block that the proof links to.
    pub(crate) async fn build_from_source_chain(
        &self,
        lower_attestation: AttestationWithProof,
        upper_attestation: AttestationWithProof,
        min_query: u64,
    ) -> ContinuityResult<(Vec<Block>, Option<H256>)> {
        let chain_blocks = self
            .build_and_trim_continuity(lower_attestation, upper_attestation, min_query)
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?;

        // Extract lower_endpoint_digest from the first block's prev_digest
        // This matches the digest of the attestation block that the proof links to
        let lower_endpoint_digest = chain_blocks.first().map(|b| b.prev_digest);

        Ok((chain_blocks, lower_endpoint_digest))
    }

    /// Build continuity blocks and trim to required range
    pub(crate) async fn build_and_trim_continuity(
        &self,
        lower: AttestationWithProof,
        upper: AttestationWithProof,
        min_query: u64,
    ) -> ContinuityResult<Vec<Block>> {
        // Continuity chain starts at queryHeight (query block at index 0)
        let required_start = min_query;

        // Determine end height (next consensus point - REQUIRED)
        // The proof MUST end at an attestation or checkpoint for verification to succeed
        let end_height = upper.block_number;

        // Determine the starting digest for build_continuity_blocks
        // build_continuity_blocks expects the digest of the block BEFORE build_start
        // With query block at index 0, the lower bound is always strictly before required_start
        // (bounds finding looks for attestation at or before min_query - 1)
        let (build_start, start_digest) = (lower.block_number + 1, lower.digest);
        assert!(
            build_start <= end_height,
            "build_start ({build_start}) must be <= end_height ({end_height})"
        );

        info!(
            build_start,
            end_height,
            required_start,
            "Building continuity chain (will trim to start at required_start)"
        );

        // Create continuity fragment
        let all_blocks: Vec<Block> = self
            .eth_provider
            .build_continuity_blocks(start_digest, build_start, end_height)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Failed to build continuity blocks: {e}")))?;

        // Trim to start at required_start (the continuity chain starts at queryHeight)
        let start_index = all_blocks
            .iter()
            .position(|b| b.block_number == required_start)
            .ok_or_else(|| {
                ContinuityError::Rpc(format!(
                    "Block {} not found in continuity chain (range: {}-{})",
                    required_start,
                    all_blocks.first().map(|b| b.block_number).unwrap_or(0),
                    all_blocks.last().map(|b| b.block_number).unwrap_or(0)
                ))
            })?;

        let trimmed = all_blocks[start_index..].to_vec();

        // Validate that trimmed blocks end at the upper attestation block
        if let Some(last_block) = trimmed.last() {
            if last_block.block_number != end_height {
                return Err(ContinuityError::Rpc(format!(
                    "Trimmed blocks don't end at upper attestation block: expected {}, got {}",
                    end_height, last_block.block_number
                )));
            }
        }

        info!(
            original_count = all_blocks.len(),
            trimmed_count = trimmed.len(),
            start_block = required_start,
            end_block = trimmed.last().map(|b| b.block_number).unwrap_or(0),
            expected_end_block = end_height,
            first_block_prev_digest = ?trimmed.first().map(|b| b.prev_digest),
            last_block_digest = ?trimmed.last().map(|b| b.digest),
            "Trimmed continuity chain"
        );

        Ok(trimmed)
    }

    /// Fetch the digest of a specific block number using build_continuity_blocks.
    /// This fetches the actual block digest from the chain.
    pub(crate) async fn fetch_block_digest(
        &self,
        block_number: u64,
        lower_attestation: &AttestationWithProof,
    ) -> ContinuityResult<H256> {
        // Special case: if block_number == lower_attestation.block_number,
        // we can't use build_continuity_blocks because we need the digest of the block before it.
        // In this case, we should have already found it in combined_blocks or indexer.
        // If we reach here, it means we couldn't find it, so we'll use the attestation digest
        // as a last resort (though this might not match what the verifier expects).
        if block_number == lower_attestation.block_number {
            tracing::warn!(
                block_number,
                "Cannot fetch attestation block digest using build_continuity_blocks, using attestation digest as fallback. This may cause verification to fail."
            );
            return Ok(lower_attestation.digest);
        }

        // Normal case: fetch from lower_attestation.block_number + 1 to block_number
        let start_digest = lower_attestation.digest;
        let build_start = lower_attestation.block_number + 1;
        let build_end = block_number;

        info!(
            block_number,
            build_start, build_end, "Fetching block digest using build_continuity_blocks"
        );

        let blocks = self
            .eth_provider
            .build_continuity_blocks(start_digest, build_start, build_end)
            .await
            .map_err(|e| {
                ContinuityError::Rpc(format!("Failed to fetch block {block_number}: {e}"))
            })?;

        let block = blocks
            .iter()
            .find(|b| b.block_number == block_number)
            .ok_or_else(|| {
                ContinuityError::Rpc(format!(
                    "Block {} not found in fetched blocks (range: {}-{})",
                    block_number,
                    blocks.first().map(|b| b.block_number).unwrap_or(0),
                    blocks.last().map(|b| b.block_number).unwrap_or(0)
                ))
            })?;

        Ok(block.digest)
    }
}
