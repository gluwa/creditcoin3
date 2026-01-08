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

        // Trim to start at required_start (the continuity chain starts at queryHeight)
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
