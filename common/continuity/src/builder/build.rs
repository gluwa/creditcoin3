use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;
use crate::errors::ContinuityError;
use crate::proof::ContinuityProof;

use anyhow::{anyhow, Context, Result};
use attestor_primitives::block::Block;
use tracing::{debug, info, warn};

impl ContinuityBuilder {
    /// Build continuity blocks and trim to required range
    async fn build_and_trim_continuity(
        &self,
        lower: AttestationInfo,
        upper: Option<AttestationInfo>,
        min_query: u64,
    ) -> Result<Vec<Block>> {
        // POC pattern: continuity chain ALWAYS starts at queryHeight - 1
        let required_start = min_query.saturating_sub(1);

        // Determine end height (next consensus point - REQUIRED)
        // The proof MUST end at an attestation or checkpoint for verification to succeed
        let end_height = upper
            .as_ref()
            .map(|u| u.block_number)
            .ok_or_else(|| anyhow!(
                "No attestation or checkpoint found after block {}. The continuity proof requires an upper bound (next attestation/checkpoint) to verify the chain ends at a consensus point.",
                min_query
            ))?;

        // Build from attestation to end to get correct digests
        // Special case: if lower bound is at required_start (e.g., block 0 checkpoint for query at block 1),
        // we need to include that block in the build, so start from lower.block_number instead of lower.block_number + 1
        let build_start = if lower.block_number == required_start {
            lower.block_number
        } else {
            lower.block_number + 1
        };

        info!(
            build_start,
            end_height,
            required_start,
            "Building continuity chain (will trim to start at required_start)"
        );

        // Create continuity fragment
        let all_blocks: Vec<Block> = self
            .eth_provider
            .build_continuity_blocks(lower.digest, build_start, end_height)
            .await
            .context("Failed to build continuity blocks")?;

        // If we built from the required start, no trimming needed
        if build_start == required_start {
            debug!(
                block_count = all_blocks.len(),
                "Generated continuity blocks"
            );
            return Ok(all_blocks);
        }

        // Trim to start at required_start
        let start_index = all_blocks
            .iter()
            .position(|b| b.block_number == required_start)
            .ok_or_else(|| {
                anyhow!(
                    "Block {} not found in continuity chain (range: {}-{})",
                    required_start,
                    all_blocks.first().map(|b| b.block_number).unwrap_or(0),
                    all_blocks.last().map(|b| b.block_number).unwrap_or(0)
                )
            })?;

        let trimmed = all_blocks[start_index..].to_vec();

        debug!(
            original_count = all_blocks.len(),
            trimmed_count = trimmed.len(),
            start_block = required_start,
            "Trimmed continuity chain"
        );

        Ok(trimmed)
    }

    /// Core logic for building continuity proof for given heights
    pub async fn build_for_heights(&self, query_heights: &[u64]) -> Result<ContinuityProof> {
        // Fetch attestations (always needed)
        let attestations = self.fetch_attestations().await?;
        if attestations.is_empty() {
            return Err(anyhow!(
                "No attestations found for chain_key {}. Queries require at least one attestation.",
                self.config.chain_key
            ));
        }

        // Find the query range
        let min_query = *query_heights.iter().min().unwrap();
        let max_query = *query_heights.iter().max().unwrap();

        // Find attestation bounds (handles queries at attestation/checkpoint heights)
        // Checkpoints are fetched lazily only when needed
        let (lower, upper) = self
            .find_attestation_bounds(min_query, max_query, &attestations)
            .await?;

        // Determine end height (next consensus point - REQUIRED)
        // The proof MUST end at an attestation or checkpoint for verification to succeed
        let end_height = upper
            .as_ref()
            .map(|u| u.block_number)
            .ok_or_else(|| anyhow!(
                "No attestation or checkpoint found after block {}. The continuity proof requires an upper bound (next attestation/checkpoint) to verify the chain ends at a consensus point.",
                min_query
            ))?;

        // Get current block height for error reporting
        let current_block = self
            .eth_provider
            .get_last_block()
            .await
            .map_err(|e| anyhow!("Failed to get current block height: {e}"))?;

        // Check if query block exists on chain
        if min_query > current_block {
            warn!(
                query_block = min_query,
                current_block, "Query block does not exist on chain yet"
            );
            return Err(ContinuityError::BlockNotReady {
                block_number: min_query,
                current_block,
            }
            .into());
        }

        // Check if upper bound (end block) is attested to
        // The upper bound is the next attestation/checkpoint after the query block
        // We need to verify this block exists and is ready
        if end_height > current_block {
            // Check if query block itself is attested to for logging
            let query_block_attested = attestations
                .iter()
                .any(|a| a.attestation.header_number <= min_query)
                || self
                    .check_if_at_checkpoint_height(min_query)
                    .await?
                    .is_some()
                || {
                    // Check if there's a checkpoint at or after the query block
                    if let Ok(Some(last_cp)) = self
                        .cc_provider
                        .get_last_checkpoint(self.config.chain_key)
                        .await
                    {
                        last_cp.block_number >= min_query
                    } else {
                        false
                    }
                };

            if !query_block_attested {
                warn!(
                    query_block = min_query,
                    current_block, "Query block is not attested to yet"
                );
            }

            warn!(
                end_block = end_height,
                current_block,
                query_block = min_query,
                "Upper bound (end block) for continuity proof is not attested to yet"
            );
            return Err(ContinuityError::BlockNotReady {
                block_number: end_height,
                current_block,
            }
            .into());
        }

        // Build and trim continuity blocks
        let blocks = self
            .build_and_trim_continuity(lower, upper, min_query)
            .await?;

        Ok(ContinuityProof::from_blocks(blocks))
    }
}
