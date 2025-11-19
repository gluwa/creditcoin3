use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;
use crate::proof::ContinuityProof;

use anyhow::{anyhow, Result};
use attestor_primitives::block::Block;
use ccnext_abi_encoding::abi::EncodingVersion;
use eth::continuity::Manager as ContinuityManager;

impl ContinuityBuilder {
    pub async fn build_and_trim_continuity(
        &self,
        lower: AttestationInfo,
        upper: Option<AttestationInfo>,
        min_query: u64,
    ) -> Result<Vec<Block>> {
        let required_start = min_query.saturating_sub(1);
        let end_height = upper
            .map(|u| u.block_number)
            .unwrap_or_else(|| min_query + 10);

        let build_start = lower.block_number + 1;

        let manager = ContinuityManager::new(build_start, end_height, &self.eth_client);
        let fragment = manager.create(lower.digest, EncodingVersion::V1).await?;

        let blocks: Vec<Block> = fragment.blocks().to_vec();

        if build_start == required_start {
            return Ok(blocks);
        }

        let idx = blocks
            .iter()
            .position(|b| b.block_number == required_start)
            .ok_or_else(|| {
                anyhow!(
                    "Block {} missing in continuity chain ({:?} → {:?})",
                    required_start,
                    blocks.first().map(|b| b.block_number),
                    blocks.last().map(|b| b.block_number)
                )
            })?;

        Ok(blocks[idx..].to_vec())
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
