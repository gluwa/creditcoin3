//! Functions for processing indexer blocks (combining, trimming, extracting digests)

use super::super::ContinuityBuilder;
use crate::builder::proof_builder::ContinuityResult;
use crate::errors::ContinuityError;
use attestor_primitives::block::Block;
use attestor_primitives::AttestationCheckpoint;
use indexer_client::AttestationWithProof;
use sp_core::H256;
use tracing::{debug, warn};

/// Checkpoint information for query processing.
struct CheckpointInfo {
    needs_checkpoint_proof: bool,
    upper_checkpoint: Option<u64>,
}

impl ContinuityBuilder {
    /// Process indexer blocks: combine if needed, trim to query range, extract digest.
    /// Returns (final_blocks, lower_endpoint_digest).
    pub(crate) async fn process_indexer_blocks(
        &self,
        indexer_blocks: Vec<Block>,
        min_query: u64,
        lower_attestation: &AttestationWithProof,
        upper_attestation: &AttestationWithProof,
    ) -> ContinuityResult<(Vec<Block>, Option<H256>)> {
        let checkpoint_info = self
            .determine_checkpoint_info(min_query, lower_attestation, upper_attestation)
            .await?;

        let mut combined_blocks = self
            .combine_blocks_for_query(
                indexer_blocks,
                min_query,
                lower_attestation,
                upper_attestation,
                &checkpoint_info,
            )
            .await?;

        combined_blocks = self
            .prepend_attestation_if_needed(combined_blocks, min_query, lower_attestation)
            .await?;

        let start_index = self.find_start_index(&combined_blocks, min_query)?;
        let lower_endpoint_digest = self
            .extract_lower_endpoint_digest(
                &combined_blocks,
                start_index,
                min_query,
                lower_attestation,
            )
            .await?;

        let trimmed = combined_blocks[start_index..].to_vec();

        // Check if upper attestation is predicted (not yet attested)
        let is_predicted = upper_attestation.digest == sp_core::H256::zero()
            && upper_attestation.continuity_proof_data.is_none();

        let final_blocks = if checkpoint_info.needs_checkpoint_proof || is_predicted {
            // For checkpoint proofs or predicted attestations, use trimmed blocks as-is
            // Upper attestation is predicted - cannot append from indexer
            // The blocks should already extend to or beyond the predicted upper bound
            // If not, we'll need to build from source chain (handled in build.rs fallback)
            trimmed
        } else {
            self.append_upper_attestation_block(trimmed, upper_attestation)
                .await?
        };

        debug!(
            final_count = final_blocks.len(),
            first_block = final_blocks.first().map(|b| b.block_number).unwrap_or(0),
            last_block = final_blocks.last().map(|b| b.block_number).unwrap_or(0),
            lower_endpoint_digest = ?lower_endpoint_digest,
            "Processed indexer blocks"
        );

        Ok((final_blocks, Some(lower_endpoint_digest)))
    }

    async fn determine_checkpoint_info(
        &self,
        min_query: u64,
        lower_attestation: &AttestationWithProof,
        upper_attestation: &AttestationWithProof,
    ) -> ContinuityResult<CheckpointInfo> {
        let last_checkpoint_block = *self.last_checkpoint_block.read().await;
        let needs_checkpoint_check = last_checkpoint_block
            .map(|last_cp| min_query <= last_cp)
            .unwrap_or(true);

        if !needs_checkpoint_check {
            return Ok(CheckpointInfo {
                needs_checkpoint_proof: false,
                upper_checkpoint: None,
            });
        }

        let checkpoints_cache = self.fetch_checkpoints_cache(min_query).await?;

        // Check if bounds are checkpoints by verifying their block numbers are checkpoint heights
        let upper_is_checkpoint = self
            .check_if_at_checkpoint_height_cached(
                upper_attestation.block_number,
                checkpoints_cache.as_deref(),
            )
            .await?;
        let lower_is_checkpoint = self
            .check_if_at_checkpoint_height_cached(
                lower_attestation.block_number,
                checkpoints_cache.as_deref(),
            )
            .await?;

        // Additional check: if root is zero and prev_digest is None, it was likely created from_checkpoint
        // This helps detect checkpoint bounds even if checkpoint cache lookup fails
        // Checkpoints created via AttestationWithProof::from_checkpoint have:
        // - root = H256::default() (zero)
        // - prev_digest = None
        let upper_is_checkpoint_from_structure = upper_attestation.root == sp_core::H256::default()
            && upper_attestation.prev_digest.is_none();
        let lower_is_checkpoint_from_structure = lower_attestation.root == sp_core::H256::default()
            && lower_attestation.prev_digest.is_none();

        let upper_looks_like_checkpoint =
            upper_is_checkpoint.is_some() || upper_is_checkpoint_from_structure;
        let lower_looks_like_checkpoint =
            lower_is_checkpoint.is_some() || lower_is_checkpoint_from_structure;

        // Bounds are checkpoints if both lower and upper are checkpoints
        // This happens when query is between checkpoints and bounds finder returns checkpoint boundaries
        // We check both checkpoint height lookup and zero root (as fallback) to detect checkpoint bounds
        let bounds_are_checkpoints = lower_looks_like_checkpoint && upper_looks_like_checkpoint;

        // Need checkpoint proof when bounds are checkpoints (query is between checkpoints OR at checkpoint)
        // When query is at checkpoint height, bounds finder returns previous checkpoint and query checkpoint
        // We build the full range from previous checkpoint to query checkpoint, including all attestations
        // When query is between checkpoints, we build from lower checkpoint to upper checkpoint
        let needs_checkpoint_proof = bounds_are_checkpoints;

        Ok(CheckpointInfo {
            needs_checkpoint_proof,
            upper_checkpoint: if bounds_are_checkpoints {
                // Bounds are checkpoints - query is between checkpoints
                // Fetch all attestations from lower checkpoint to upper checkpoint
                Some(upper_attestation.block_number)
            } else {
                // Query is not between checkpoints - don't end at checkpoint
                None
            },
        })
    }

    async fn fetch_checkpoints_cache(
        &self,
        min_query: u64,
    ) -> ContinuityResult<Option<Vec<AttestationCheckpoint>>> {
        if let Some(ref indexer) = self.indexer_provider {
            let max_range = self.config.checkpoint_query_max_range();
            Ok(Some(
                indexer
                    .get_checkpoints_around_height(self.config.chain_key, min_query, max_range)
                    .await
                    .map_err(|e| {
                        ContinuityError::Rpc(format!(
                            "Failed to fetch checkpoints around height: {e}"
                        ))
                    })?,
            ))
        } else {
            Ok(Some(
                self.fetch_checkpoints_smart(None, None)
                    .await
                    .map_err(|e| {
                        ContinuityError::Rpc(format!("Failed to fetch checkpoints: {e}"))
                    })?,
            ))
        }
    }

    async fn combine_blocks_for_query(
        &self,
        indexer_blocks: Vec<Block>,
        min_query: u64,
        lower_attestation: &AttestationWithProof,
        upper_attestation: &AttestationWithProof,
        checkpoint_info: &CheckpointInfo,
    ) -> ContinuityResult<Vec<Block>> {
        if checkpoint_info.needs_checkpoint_proof {
            let upper_checkpoint = checkpoint_info
                .upper_checkpoint
                .unwrap_or(upper_attestation.block_number);

            if let Some(chained_blocks) = self
                .try_build_checkpoint_spanning_proof(min_query, lower_attestation, upper_checkpoint)
                .await?
            {
                return Ok(chained_blocks);
            }
            warn!("Failed to build checkpoint-spanning proof - using single attestation");
        }

        if indexer_blocks.iter().any(|b| b.block_number == min_query) {
            return Ok(indexer_blocks);
        }

        if let Some(chained_blocks) = self
            .try_build_by_chaining_attestations(min_query, upper_attestation)
            .await?
        {
            return Ok(chained_blocks);
        }

        self.combine_attestation_proofs_if_needed(indexer_blocks, min_query)
            .await
    }

    async fn prepend_attestation_if_needed(
        &self,
        combined_blocks: Vec<Block>,
        min_query: u64,
        lower_attestation: &AttestationWithProof,
    ) -> ContinuityResult<Vec<Block>> {
        // Prepend lower checkpoint/attestation block if:
        // 1. Query is at lower_attestation + 1 (normal case)
        // 2. Query is at lower_attestation (checkpoint height query - need to include lower checkpoint for proper digest chain)
        let should_prepend = min_query == lower_attestation.block_number + 1
            || min_query == lower_attestation.block_number;

        if !should_prepend {
            return Ok(combined_blocks);
        }

        if combined_blocks
            .iter()
            .any(|b| b.block_number == lower_attestation.block_number)
        {
            return Ok(combined_blocks);
        }

        let Some(ref indexer) = self.indexer_provider else {
            return Ok(combined_blocks);
        };

        debug!(
            min_query,
            lower_attestation_block = lower_attestation.block_number,
            "Prepending attestation block for lower_endpoint_digest"
        );

        let Some(attestation_with_proof) = indexer
            .get_continuity_blocks(self.config.chain_key, lower_attestation.block_number)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Failed to fetch continuity blocks: {e}")))?
        else {
            warn!("Failed to fetch attestation - will handle in extract_lower_endpoint_digest");
            return Ok(combined_blocks);
        };

        let continuity_blocks = attestation_with_proof
            .extract_blocks()
            .map_err(|e| ContinuityError::Rpc(format!("Failed to extract blocks: {e}")))?;

        let prev_digest = continuity_blocks
            .and_then(|blocks| blocks.last().map(|b| b.digest))
            .or(lower_attestation.prev_digest);

        let mut attestation_block_vec = Vec::new();
        self.add_attestation_block(
            &mut attestation_block_vec,
            lower_attestation.block_number,
            attestation_with_proof.root,
            prev_digest,
        );

        if let Some(attestation_block) = attestation_block_vec.first() {
            let mut new_combined = vec![attestation_block.clone()];
            new_combined.extend(combined_blocks);
            Ok(new_combined)
        } else {
            Ok(combined_blocks)
        }
    }

    fn find_start_index(&self, blocks: &[Block], min_query: u64) -> ContinuityResult<usize> {
        blocks
            .iter()
            .position(|b| b.block_number == min_query)
            .ok_or_else(|| {
                let block_numbers: Vec<u64> = blocks.iter().map(|b| b.block_number).collect();
                warn!(
                    min_query,
                    block_count = blocks.len(),
                    first = blocks.first().map(|b| b.block_number).unwrap_or(0),
                    last = blocks.last().map(|b| b.block_number).unwrap_or(0),
                    block_numbers = ?block_numbers,
                    "Query block not found"
                );
                ContinuityError::Rpc(format!(
                    "Query block {} not found in blocks (range: {}-{})",
                    min_query,
                    blocks.first().map(|b| b.block_number).unwrap_or(0),
                    blocks.last().map(|b| b.block_number).unwrap_or(0)
                ))
            })
    }

    /// Extract the lower endpoint digest (digest of block min_query - 1).
    async fn extract_lower_endpoint_digest(
        &self,
        combined_blocks: &[Block],
        start_index: usize,
        min_query: u64,
        lower_attestation: &AttestationWithProof,
    ) -> ContinuityResult<H256> {
        let target_block = min_query - 1;

        // Special case: Query is at attestation + 1
        if min_query == lower_attestation.block_number + 1 {
            if let Some(digest) = self.find_target_block_in_blocks(combined_blocks, target_block) {
                return Ok(digest);
            }
            warn!(
                min_query,
                target_block, "Attestation block not found - using attestation digest as fallback"
            );
            return Ok(lower_attestation.digest);
        }

        // Special case: Query is exactly at attestation
        if min_query == lower_attestation.block_number {
            return self
                .fetch_target_block_from_indexer(target_block, lower_attestation)
                .await;
        }

        // Normal case: Try to find target block in combined_blocks
        if let Some(digest) = self.find_target_block_in_blocks(combined_blocks, target_block) {
            return Ok(digest);
        }

        if start_index > 0 {
            let prev_block = &combined_blocks[start_index - 1];
            if prev_block.block_number == target_block {
                return Ok(prev_block.digest);
            }
        }

        self.fetch_target_block_from_indexer(target_block, lower_attestation)
            .await
    }

    fn find_target_block_in_blocks(&self, blocks: &[Block], target_block: u64) -> Option<H256> {
        blocks
            .iter()
            .find(|b| b.block_number == target_block)
            .map(|b| b.digest)
    }

    async fn fetch_target_block_from_indexer(
        &self,
        target_block: u64,
        lower_attestation: &AttestationWithProof,
    ) -> ContinuityResult<H256> {
        let Some(ref indexer) = self.indexer_provider else {
            return self
                .fetch_block_digest(target_block, lower_attestation)
                .await;
        };

        // Try getting continuity blocks for the lower attestation
        if let Ok(Some(attestation_with_proof)) = indexer
            .get_continuity_blocks(self.config.chain_key, lower_attestation.block_number)
            .await
        {
            if let Some(blocks) = attestation_with_proof
                .extract_blocks()
                .map_err(|e| ContinuityError::Rpc(format!("Failed to extract blocks: {e}")))?
            {
                if let Some(block) = blocks.iter().find(|b| b.block_number == target_block) {
                    return Ok(block.digest);
                }
            }

            // Try previous attestation
            let prev_block = lower_attestation.block_number.saturating_sub(1);
            if let Ok(Some(prev_att)) = indexer
                .find_attestation_before_or_at(self.config.chain_key, prev_block)
                .await
            {
                if let Some(prev_blocks) = prev_att
                    .extract_blocks()
                    .map_err(|e| ContinuityError::Rpc(format!("Failed to extract blocks: {e}")))?
                {
                    if let Some(block) = prev_blocks.iter().find(|b| b.block_number == target_block)
                    {
                        return Ok(block.digest);
                    }
                }
            }
        }

        Err(ContinuityError::Rpc(format!(
            "Block {target_block} (min_query - 1) not found in indexer"
        )))
    }

    /// Append the upper attestation block if it's not already included.
    pub(crate) async fn append_upper_attestation_block(
        &self,
        blocks: Vec<Block>,
        upper_attestation: &AttestationWithProof,
    ) -> ContinuityResult<Vec<Block>> {
        let Some(ref indexer) = self.indexer_provider else {
            return Ok(blocks);
        };

        if blocks
            .last()
            .map(|b| b.block_number == upper_attestation.block_number)
            .unwrap_or(false)
        {
            return Ok(blocks);
        }

        let attestation = indexer
            .get_attestation(self.config.chain_key, upper_attestation.block_number)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Failed to fetch attestation data: {e}")))?
            .ok_or_else(|| {
                ContinuityError::Rpc(format!(
                    "Attestation not found at block {}",
                    upper_attestation.block_number
                ))
            })?;

        let last_block_digest = blocks.last().map(|b| b.digest);
        let mut new_blocks = blocks;
        self.add_attestation_block(
            &mut new_blocks,
            upper_attestation.block_number,
            attestation.root,
            last_block_digest,
        );

        Ok(new_blocks)
    }

    /// Check if a block height is at a checkpoint, using cached checkpoints if available.
    pub(crate) async fn check_if_at_checkpoint_height_cached(
        &self,
        query_height: u64,
        checkpoints_cache: Option<&[AttestationCheckpoint]>,
    ) -> ContinuityResult<Option<AttestationCheckpoint>> {
        if let Some(cached) = checkpoints_cache {
            return Ok(cached
                .iter()
                .find(|c| c.block_number == query_height)
                .cloned());
        }

        self.check_if_at_checkpoint_height(query_height)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Failed to check checkpoint height: {e}")))
    }
}
