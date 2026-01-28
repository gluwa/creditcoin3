//! Indexer-based bounds finding
//!
//! This module finds attestation/checkpoint bounds using GraphQL indexer queries.
//! This is faster than querying the CC3 chain directly.

use super::BoundsFinder;
use crate::builder::ContinuityBuilder;
use crate::errors::ContinuityError;
use async_trait::async_trait;
use indexer_client::{AttestationWithProof, IndexerClient};
use tracing::info;

/// Bounds finder that uses the GraphQL indexer.
pub struct IndexerBoundsFinder<'a> {
    builder: &'a ContinuityBuilder,
    indexer: &'a IndexerClient,
}

impl<'a> IndexerBoundsFinder<'a> {
    pub fn new(builder: &'a ContinuityBuilder, indexer: &'a IndexerClient) -> Self {
        Self { builder, indexer }
    }
}

#[async_trait]
impl<'a> BoundsFinder for IndexerBoundsFinder<'a> {
    async fn find_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        current_block: Option<u64>,
    ) -> Result<(AttestationWithProof, AttestationWithProof), ContinuityError> {
        info!(
            chain_key = self.builder.config.chain_key,
            min_query, max_query, "Using indexer for attestation endpoint lookup"
        );

        // OPTIMIZATION: First check the last checkpoint - if query is after it, skip checkpoint checks
        let last_checkpoint = self
            .indexer
            .get_last_checkpoint(self.builder.config.chain_key)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Failed to fetch last checkpoint: {e}")))?;

        let checkpoints = if let Some(ref last_cp) = last_checkpoint {
            if min_query > last_cp.block_number {
                // Query is after last checkpoint - no need to fetch more checkpoints
                info!(
                    min_query,
                    last_checkpoint_block = last_cp.block_number,
                    "Query is after last checkpoint - skipping checkpoint boundary check"
                );
                Vec::new()
            } else {
                // Query is before or at last checkpoint - fetch checkpoints around query height
                let max_range = self.builder.config.checkpoint_query_max_range();
                self.indexer
                    .get_checkpoints_around_height(
                        self.builder.config.chain_key,
                        min_query,
                        max_range,
                    )
                    .await
                    .map_err(|e| {
                        ContinuityError::Rpc(format!(
                            "Failed to fetch checkpoints around height: {e}"
                        ))
                    })?
            }
        } else {
            // No checkpoints exist - skip checkpoint checks
            Vec::new()
        };

        // Find checkpoint strictly before query (closest checkpoint < min_query)
        // CRITICAL: When query is exactly at a checkpoint, we need the PREVIOUS checkpoint
        // as the lower bound to satisfy validation: min_query > lower_attestation.block_number
        let checkpoint_before = checkpoints
            .iter()
            .filter(|c| c.block_number < min_query)
            .max_by_key(|c| c.block_number)
            .cloned();

        // Find checkpoint at or after query (smallest checkpoint >= max_query)
        // CRITICAL: Use max_query (not min_query) to ensure bounds cover entire query range
        let checkpoint_after = checkpoints
            .iter()
            .filter(|c| c.block_number >= max_query)
            .min_by_key(|c| c.block_number)
            .cloned();

        // If query is between checkpoints (has checkpoint before and after), use checkpoint boundaries
        if let (Some(cp_before), Some(cp_after)) = (checkpoint_before, checkpoint_after) {
            info!(
                lower_bound = cp_before.block_number,
                upper_bound = cp_after.block_number,
                "Query is between checkpoints - using checkpoint boundaries"
            );
            return Ok((
                AttestationWithProof::from_checkpoint(&cp_before),
                AttestationWithProof::from_checkpoint(&cp_after),
            ));
        }

        // Otherwise, use attestation boundaries (original logic)
        // CRITICAL: Check if query is between checkpoints first
        // If so, use checkpoint boundaries instead of attestation boundaries
        let required_before = min_query.saturating_sub(1);
        let lower = self
            .indexer
            .find_attestation_before_or_at(self.builder.config.chain_key, required_before)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Indexer error: {e}")))?
            .ok_or_else(|| {
                ContinuityError::Rpc(format!(
                    "No attestation found before block {required_before} (query height: {min_query})"
                ))
            })?;

        // Find upper bound (attestation after max_query)
        let upper_opt = self
            .indexer
            .find_attestation_after(self.builder.config.chain_key, max_query)
            .await
            .map_err(|e| ContinuityError::Rpc(format!("Indexer error: {e}")))?;

        match upper_opt {
            Some(u) => {
                info!(
                    lower_bound = lower.block_number,
                    upper_bound = u.block_number,
                    "Found attestation bounds via indexer"
                );
                Ok((lower, u))
            }
            None => {
                // No upper attestation found - predict next one
                let predicted = self.builder.predict_next_attestation(max_query).await?;
                self.builder.validate_predicted_upper_bound(
                    predicted.block_number,
                    max_query,
                    current_block,
                )?;

                info!(
                    lower_bound = lower.block_number,
                    predicted_upper = predicted.block_number,
                    "Using predicted upper bound (no attestation found after query)"
                );
                Ok((lower, predicted))
            }
        }
    }
}
