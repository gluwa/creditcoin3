use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;
use crate::proof::ContinuityProof;

use anyhow::{anyhow, Context, Result};
use attestor_primitives::block::Block;

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

        // Determine end height (next consensus point or fallback)
        let end_height = upper
            .map(|u| u.block_number)
            .unwrap_or_else(|| min_query + 10);

        // Build from attestation to end to get correct digests
        let build_start = lower.block_number + 1;

        println!(
            "Building continuity chain from {build_start} to {end_height} (will trim to start at {required_start})"
        );

        // Create continuity fragment
        let all_blocks: Vec<Block> = self
            .eth_provider
            .build_continuity_blocks(lower.digest, build_start, end_height)
            .await
            .context("Failed to build continuity blocks")?;

        // If we built from the required start, no trimming needed
        if build_start == required_start {
            println!("Generated {} continuity blocks", all_blocks.len());
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

        println!(
            "Trimmed continuity chain from {} to {} blocks (starting at block {})",
            all_blocks.len(),
            trimmed.len(),
            required_start
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

        // Build and trim continuity blocks
        let blocks = self
            .build_and_trim_continuity(lower, upper, min_query)
            .await?;

        Ok(ContinuityProof::from_blocks(blocks))
    }
}
