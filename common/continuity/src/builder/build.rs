use super::ContinuityBuilder;
use crate::proof::ContinuityProof;
use crate::{attestation::AttestationInfo, builder::EndsInAttestation};

use anyhow::{anyhow, bail, Context, Result};
use attestor_primitives::block::Block;
use tracing::{debug, info};

impl ContinuityBuilder {
    /// Build continuity blocks and trim to required range
    async fn build_and_trim_continuity(
        &self,
        lower: AttestationInfo,
        upper: AttestationInfo,
        min_query: u64,
    ) -> Result<Vec<Block>> {
        // POC pattern: continuity chain ALWAYS starts at queryHeight - 1
        let required_start = min_query.saturating_sub(1);

        // Determine end height (next consensus point - REQUIRED)
        // The proof MUST end at an attestation or checkpoint for verification to succeed
        let end_height = upper.block_number;

        // Determine the starting digest for build_continuity_blocks
        // build_continuity_blocks expects the digest of the block BEFORE build_start
        let (build_start, start_digest) = if lower.block_number == required_start {
            // Special case: lower attestation/checkpoint is at required_start
            match lower.prev_digest {
                Some(prev_digest) => {
                    // Attestation case: prev_digest is available
                    (required_start, prev_digest)
                }
                None => {
                    // Checkpoint case: need to find previous checkpoint and build continuity
                    // to get the digest of block (checkpoint_height - 1)
                    // Special case: if checkpoint is at block 0, use zero digest (genesis block)
                    if lower.block_number == 0 {
                        // Genesis checkpoint: use zero digest as lower_endpoint_digest
                        (required_start, sp_core::H256::default())
                    } else {
                        let checkpoints = self
                            .cc_provider
                            .get_checkpoints_for_chain(self.config.chain_key)
                            .await
                            .ok();
                        let attestations = self.fetch_attestations().await.ok().unwrap_or_default();

                        // Find previous checkpoint before lower.block_number
                        let prev_checkpoint = checkpoints.as_ref().and_then(|cps| {
                            cps.iter()
                                .filter(|c| c.block_number < lower.block_number)
                                .max_by_key(|c| c.block_number)
                        });

                        // Find previous attestation before lower.block_number
                        let prev_attestation = attestations
                            .iter()
                            .filter(|a| a.attestation.header_number < lower.block_number)
                            .max_by_key(|a| a.attestation.header_number);

                        // Use the closest previous consensus point (highest block number)
                        let (prev_block_number, prev_digest) = match (
                            prev_checkpoint,
                            prev_attestation,
                        ) {
                            (Some(c), Some(a)) => {
                                if c.block_number > a.attestation.header_number {
                                    (c.block_number, c.digest)
                                } else {
                                    (a.attestation.header_number, a.attestation.digest())
                                }
                            }
                            (Some(c), None) => (c.block_number, c.digest),
                            (None, Some(a)) => {
                                (a.attestation.header_number, a.attestation.digest())
                            }
                            (None, None) => {
                                return Err(anyhow!(
                                    "No previous checkpoint or attestation found before checkpoint at block {}. Cannot build continuity proof.",
                                    lower.block_number
                                ));
                            }
                        };

                        // Build continuity from previous consensus point to (checkpoint_height - 1)
                        // to get the digest of block (checkpoint_height - 1)
                        let target_height = lower.block_number.saturating_sub(1);
                        let prev_block_digest = if prev_block_number == target_height {
                            // Previous consensus point is exactly at target_height, use its digest directly
                            prev_digest
                        } else {
                            // Build continuity chain from previous consensus point to target_height
                            let intermediate_blocks = self
                                .eth_provider
                                .build_continuity_blocks(
                                    prev_digest,
                                    prev_block_number + 1,
                                    target_height,
                                )
                                .await
                                .context(
                                    "Failed to build intermediate continuity blocks to get prev_digest",
                                )?;

                            // Get the digest of the last block (target_height = checkpoint_height - 1)
                            intermediate_blocks
                                .last()
                                .map(|b| b.digest)
                            .ok_or_else(|| {
                                anyhow!(
                                    "Failed to get digest of block {target_height} (checkpoint_height - 1)"
                                )
                            })?
                        };

                        // Build from required_start (checkpoint height), using digest of block (checkpoint_height - 1)
                        (required_start, prev_block_digest)
                    }
                }
            }
        } else {
            // Normal case: build from lower.block_number + 1, use lower.digest (digest of lower.block_number)
            (lower.block_number + 1, lower.digest)
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
            .build_continuity_blocks(start_digest, build_start, end_height)
            .await
            .context("Failed to build continuity blocks")?;

        // Trim to start at required_start (the continuity chain must start at queryHeight - 1)
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
    pub async fn build_for_heights(
        &self,
        query_heights: &[u64],
        lower_attestation: AttestationInfo,
        upper_attestation: AttestationInfo,
    ) -> Result<ContinuityProof> {
        // Find the query range
        let min_query = *query_heights
            .iter()
            .min()
            .ok_or(anyhow!("query_heights has 0 entries."))?;
        let max_query = *query_heights
            .iter()
            .max()
            .ok_or(anyhow!("query_heights has 0 entries."))?;

        // Verify that parameters are valid.
        // The query heights are contained within the attestation bounds.
        if max_query > upper_attestation.block_number || min_query <= lower_attestation.block_number
        {
            bail!(
                "Query heights not contained by attestation bounds! min_query: {min_query}, max_query: {max_query}, lower_attestation: {}, upper_attestation: {}",
                lower_attestation.block_number,
                upper_attestation.block_number,
            );
        }

        // Build and trim continuity blocks
        let blocks = self
            .build_and_trim_continuity(lower_attestation, upper_attestation, min_query)
            .await?;

        Ok(ContinuityProof::from_blocks(blocks))
    }

    pub async fn get_endpoints(
        &self,
        query_heights: &[u64],
    ) -> Result<(AttestationInfo, AttestationInfo, EndsInAttestation)> {
        // Find the query range
        let min_query = *query_heights
            .iter()
            .min()
            .ok_or(anyhow!("query_heights has 0 entries."))?;
        let max_query = *query_heights
            .iter()
            .max()
            .ok_or(anyhow!("query_heights has 0 entries."))?;

        // Fetch attestations (always needed)
        let attestations = self.fetch_attestations().await?;
        if attestations.is_empty() {
            bail!(
                "No attestations found for chain_key {}. Queries require at least one attestation.",
                self.config.chain_key
            );
        }

        // Find attestation bounds (handles queries at attestation/checkpoint heights)
        // Checkpoints are fetched lazily only when needed
        let (lower, upper, ends_in_attestation) = self
            .find_attestation_bounds(min_query, max_query, &attestations)
            .await?;

        // Determine end height (next consensus point - REQUIRED)
        // The proof MUST end at an attestation or checkpoint for verification to succeed
        let upper = upper
            .ok_or_else(|| anyhow!(
                "No attestation or checkpoint found after block {max_query}. The continuity proof requires an upper bound (next attestation/checkpoint) to verify the chain ends at a consensus point."
            ))?;

        Ok((lower, upper, ends_in_attestation))
    }
}
