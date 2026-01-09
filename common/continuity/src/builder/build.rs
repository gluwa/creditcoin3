use super::ContinuityBuilder;
use crate::errors::ContinuityError;
use crate::proof::ContinuityProof;
use crate::{attestation::AttestationInfo, builder::EndsInAttestation};

use anyhow::{anyhow, Context, Result};
use attestor_primitives::block::Block;
use sp_core::H256;
use tracing::{debug, info};

/// Result type for continuity builder operations that return typed errors.
pub type ContinuityResult<T> = Result<T, ContinuityError>;

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
    ) -> ContinuityResult<ContinuityProof> {
        // Find the query range
        let min_query = *query_heights
            .iter()
            .min()
            .ok_or(ContinuityError::EmptyQuery)?;
        let max_query = *query_heights
            .iter()
            .max()
            .ok_or(ContinuityError::EmptyQuery)?;

        // Verify that parameters are valid.
        // The query heights are contained within the attestation bounds.
        if max_query > upper_attestation.block_number || min_query <= lower_attestation.block_number
        {
            return Err(ContinuityError::InvalidBounds(format!(
                "Query heights not contained by attestation bounds! min_query: {min_query}, max_query: {max_query}, lower_attestation: {}, upper_attestation: {}",
                lower_attestation.block_number,
                upper_attestation.block_number,
            )));
        }

        // Build and trim continuity blocks
        let blocks = self
            .build_and_trim_continuity(lower_attestation, upper_attestation, min_query)
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?;

        Ok(ContinuityProof::from_blocks(blocks))
    }

    pub async fn get_endpoints(
        &self,
        query_heights: &[u64],
    ) -> ContinuityResult<(AttestationInfo, AttestationInfo, EndsInAttestation)> {
        // Find the query range
        let min_query = *query_heights
            .iter()
            .min()
            .ok_or(ContinuityError::EmptyQuery)?;
        let max_query = *query_heights
            .iter()
            .max()
            .ok_or(ContinuityError::EmptyQuery)?;

        // Fetch attestations (always needed)
        let attestations = self
            .fetch_attestations()
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?;
        if attestations.is_empty() {
            return Err(ContinuityError::NoAttestations(self.config.chain_key));
        }

        // Find attestation bounds (handles queries at attestation/checkpoint heights)
        // Checkpoints are fetched lazily only when needed
        let (lower, upper, ends_in_attestation) = self
            .find_attestation_bounds(min_query, max_query, &attestations)
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?;

        // If no upper bound exists (block not yet attested), predict the next attestation
        // using the attestation interval. This enables "eager" proof generation where
        // proofs can be created before the attestation exists.
        let (upper, ends_in_attestation) = match upper {
            Some(u) => (u, ends_in_attestation),
            None => {
                let predicted = self.predict_next_attestation(max_query).await?;
                info!(
                    max_query,
                    predicted_upper = predicted.block_number,
                    "No attestation found after query block - using predicted upper bound"
                );
                // Predicted upper bounds don't end in an attestation yet
                (predicted, EndsInAttestation::False)
            }
        };

        Ok((lower, upper, ends_in_attestation))
    }

    /// Predict the next attestation block number based on the attestation interval.
    /// This is used for "eager" proof generation when a block is not yet attested.
    ///
    /// The formula aligns to the attestation interval boundary:
    /// `next_attestation = ((block / interval) + 1) * interval`
    async fn predict_next_attestation(&self, block: u64) -> ContinuityResult<AttestationInfo> {
        let interval = self
            .cc_provider
            .get_attestation_interval(self.config.chain_key)
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?
            .ok_or(ContinuityError::AttestationIntervalNotConfigured {
                chain_key: self.config.chain_key,
            })?;

        // Calculate the next attestation block aligned to the interval
        // e.g., if block=25 and interval=10, next attestation is at block 30
        let next_attestation_block = ((block / interval) + 1) * interval;

        debug!(
            query_block = block,
            interval,
            predicted_attestation = next_attestation_block,
            "Predicted next attestation block"
        );

        // Return a predicted AttestationInfo with a zero digest (will be computed from source chain blocks)
        // The digest field is not used for the upper bound in build_and_trim_continuity
        Ok(AttestationInfo {
            block_number: next_attestation_block,
            digest: H256::default(),
            prev_digest: None,
        })
    }
}
