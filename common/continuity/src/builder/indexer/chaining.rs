//! Functions for chaining attestations from indexer data

use super::super::ContinuityBuilder;
use crate::builder::proof_builder::ContinuityResult;
use crate::errors::ContinuityError;
use attestor_primitives::block::Block;
use indexer_client::AttestationWithProof;
use tracing::{debug, warn};

impl ContinuityBuilder {
    /// Build checkpoint-spanning proof by chaining attestations from query to upper checkpoint.
    /// Uses optimized batch query to fetch all attestations in one request.
    pub(crate) async fn try_build_checkpoint_spanning_proof(
        &self,
        min_query: u64,
        lower_attestation: &AttestationWithProof,
        upper_checkpoint: u64,
    ) -> ContinuityResult<Option<Vec<Block>>> {
        let Some(ref indexer) = self.indexer_provider else {
            return Ok(None);
        };

        debug!(
            min_query,
            lower_bound = lower_attestation.block_number,
            upper_checkpoint,
            "Building checkpoint-spanning proof with batched query"
        );

        let fetch_start = lower_attestation.block_number + 1;
        let attestations = indexer
            .get_attestations_in_range(self.config.chain_key, fetch_start, upper_checkpoint)
            .await
            .map_err(|e| {
                warn!(error = %e, "Failed to fetch attestations in range");
                ContinuityError::Rpc(format!("Failed to fetch attestations: {e}"))
            })?;

        if attestations.is_empty() {
            warn!(
                min_query,
                upper_checkpoint, "No attestations found in checkpoint range"
            );
            return Ok(None);
        }

        debug!(
            attestation_count = attestations.len(),
            first_attestation = attestations.first().map(|a| a.block_number).unwrap_or(0),
            last_attestation = attestations.last().map(|a| a.block_number).unwrap_or(0),
            "Fetched attestations in range"
        );

        // Process all attestations: extract continuity blocks and add attestation blocks
        let mut all_blocks = Vec::new();
        for attestation in attestations {
            self.process_single_attestation(&mut all_blocks, &attestation)
                .await?;
        }

        if all_blocks.is_empty() {
            warn!("No blocks built from attestations in checkpoint range");
            return Ok(None);
        }

        debug!(
            total_blocks = all_blocks.len(),
            first = all_blocks.first().map(|b| b.block_number).unwrap_or(0),
            last = all_blocks.last().map(|b| b.block_number).unwrap_or(0),
            "Built checkpoint-spanning proof"
        );

        Ok(Some(all_blocks))
    }

    /// Build continuity proof by chaining attestations from query block to target block.
    /// Returns None if we can't build from indexer.
    pub(crate) async fn try_build_by_chaining_attestations(
        &self,
        min_query: u64,
        upper_attestation: &AttestationWithProof,
    ) -> ContinuityResult<Option<Vec<Block>>> {
        let Some(ref indexer) = self.indexer_provider else {
            return Ok(None);
        };

        debug!(
            min_query,
            upper_attestation = upper_attestation.block_number,
            "Chaining attestations for non-checkpoint query"
        );

        // Find the first attestation >= min_query
        let mut current = match indexer
            .find_attestation_after(self.config.chain_key, min_query.saturating_sub(1))
            .await
        {
            Ok(Some(att)) => att,
            Ok(None) | Err(_) => {
                warn!("Could not find first attestation for chaining");
                return Ok(None);
            }
        };

        let mut all_blocks = Vec::new();
        const MAX_ITERATIONS: usize = 100;
        let mut iteration = 0;

        while iteration <= MAX_ITERATIONS {
            iteration += 1;

            // Process current attestation: extract blocks and add attestation block
            self.process_single_attestation(&mut all_blocks, &current)
                .await?;

            // Check if we've reached the target
            if current.block_number >= upper_attestation.block_number {
                break;
            }

            // Find next attestation
            match indexer
                .find_attestation_after(self.config.chain_key, current.block_number)
                .await
            {
                Ok(Some(next)) if next.block_number > current.block_number => {
                    if next.block_number > upper_attestation.block_number {
                        // Next is beyond target - use upper_attestation if needed
                        current = self
                            .get_attestation_with_proof(indexer, upper_attestation)
                            .await?
                            .unwrap_or(current);
                        if current.block_number >= upper_attestation.block_number {
                            break;
                        }
                    } else {
                        current = next;
                    }
                }
                Ok(None) => {
                    // Try upper_attestation as fallback
                    if current.block_number < upper_attestation.block_number {
                        current = self
                            .get_attestation_with_proof(indexer, upper_attestation)
                            .await?
                            .unwrap_or(current);
                        if current.block_number >= upper_attestation.block_number {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                Err(e) => {
                    // Bubble up error instead of silently breaking
                    return Err(ContinuityError::Rpc(format!(
                        "Failed to find next attestation: {e}"
                    )));
                }
                _ => break, // Not advancing
            }
        }

        if iteration > MAX_ITERATIONS {
            warn!(
                iterations = iteration,
                current = current.block_number,
                "Hit maximum iterations - possible infinite loop"
            );
            return Ok(None);
        }

        if all_blocks.is_empty() {
            return Ok(None);
        }

        debug!(
            total_blocks = all_blocks.len(),
            first_block = all_blocks.first().map(|b| b.block_number).unwrap_or(0),
            last_block = all_blocks.last().map(|b| b.block_number).unwrap_or(0),
            "Built proof by chaining attestations"
        );

        Ok(Some(all_blocks))
    }

    /// Combine attestation continuity proofs when query is at attestation height.
    /// Fetches the attestation at min_query and prepends its blocks to the current blocks.
    pub(crate) async fn combine_attestation_proofs_if_needed(
        &self,
        indexer_blocks: Vec<Block>,
        min_query: u64,
    ) -> ContinuityResult<Vec<Block>> {
        // Early return if query block is already in blocks
        if indexer_blocks.iter().any(|b| b.block_number == min_query) {
            return Ok(indexer_blocks);
        }

        let Some(ref indexer) = self.indexer_provider else {
            return Ok(indexer_blocks);
        };

        debug!(
            min_query,
            indexer_first = indexer_blocks.first().map(|b| b.block_number).unwrap_or(0),
            "Query block not in fetched blocks - fetching attestation at query height"
        );

        let Some(attestation_with_proof) = indexer
            .get_continuity_blocks(self.config.chain_key, min_query)
            .await
            .ok()
            .flatten()
        else {
            return Ok(indexer_blocks);
        };

        let Some(at_query_blocks) = self.extract_blocks_safe(&attestation_with_proof).await? else {
            return Ok(indexer_blocks);
        };

        // Check if attestation ends at query height (can combine)
        if at_query_blocks.last().map(|b| b.block_number) == Some(min_query) {
            debug!("Attestation ends at query height - combining with upper blocks");
            let mut combined = at_query_blocks;
            combined.extend(indexer_blocks);
            Ok(combined)
        } else {
            warn!(
                expected_last = min_query,
                actual_last = at_query_blocks.last().map(|b| b.block_number),
                "Fetched attestation doesn't end at query height - cannot combine"
            );
            Ok(indexer_blocks)
        }
    }

    // Helper: Process a single attestation (extract continuity blocks + add attestation block)
    async fn process_single_attestation(
        &self,
        all_blocks: &mut Vec<Block>,
        attestation: &AttestationWithProof,
    ) -> ContinuityResult<()> {
        let header_number = attestation.block_number;

        // Extract and add continuity blocks
        if let Some(blocks) = self.extract_blocks_safe(attestation).await? {
            debug!(
                attestation = header_number,
                block_count = blocks.len(),
                "Adding continuity blocks for attestation"
            );
            all_blocks.extend(blocks);
        } else {
            debug!(
                attestation = header_number,
                "Attestation has no continuity blocks"
            );
        }

        // Add the attestation block itself
        self.add_attestation_block(
            all_blocks,
            header_number,
            attestation.root,
            attestation.prev_digest,
        );

        Ok(())
    }

    // Helper: Extract blocks from attestation with safe error handling
    async fn extract_blocks_safe(
        &self,
        attestation: &AttestationWithProof,
    ) -> ContinuityResult<Option<Vec<Block>>> {
        attestation
            .extract_blocks()
            .map_err(|e| ContinuityError::Rpc(format!("Failed to extract blocks: {e}")))
    }

    // Helper: Get attestation with proof, using cached data if available
    async fn get_attestation_with_proof(
        &self,
        indexer: &indexer_client::IndexerClient,
        attestation: &AttestationWithProof,
    ) -> ContinuityResult<Option<AttestationWithProof>> {
        if attestation.continuity_proof_data.is_some() {
            return Ok(Some(attestation.clone()));
        }

        indexer
            .get_continuity_blocks(self.config.chain_key, attestation.block_number)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Failed to fetch continuity blocks: {e}")))
    }
}
