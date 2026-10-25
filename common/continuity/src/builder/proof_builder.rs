//! Continuity proof building logic
//!
//! This module orchestrates continuity proof building by delegating to specialized modules:
//! - `bounds/`: Finding attestation/checkpoint bounds (indexer or CC3 chain)
//! - `indexer/`: Indexer-specific proof building logic
//! - `cc3/`: CC3 chain-specific proof building logic
//! - `common/`: Shared utilities

use super::ContinuityBuilder;
use crate::errors::ContinuityError;
use crate::proof::BuiltContinuityProof;
use indexer_client::AttestationWithProof;
use sp_core::H256;
use tracing::{debug, info};

/// Result type for continuity builder operations that return typed errors.
pub type ContinuityResult<T> = Result<T, ContinuityError>;

impl ContinuityBuilder {
    /// Core logic for building continuity proof for given heights
    pub async fn build_for_heights(
        &self,
        query_heights: &[u64],
        lower_attestation: AttestationWithProof,
        upper_attestation: AttestationWithProof,
    ) -> ContinuityResult<BuiltContinuityProof> {
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

        // Try to get blocks from indexer first (if available)
        let (blocks, lower_endpoint_digest) = if let Some(ref indexer) = self.indexer_provider {
            // If upper attestation is predicted (not yet attested), query lower attestation instead
            // Predicted attestations have zero digest and no continuity_proof_data
            let is_predicted = upper_attestation.digest == H256::zero()
                && upper_attestation.continuity_proof_data.is_none();

            let query_block = if is_predicted {
                info!(
                    upper_attestation = upper_attestation.block_number,
                    lower_attestation = lower_attestation.block_number,
                    min_query,
                    "Upper attestation is predicted - querying lower attestation continuity blocks from indexer"
                );
                lower_attestation.block_number
            } else {
                info!(
                    upper_attestation = upper_attestation.block_number,
                    min_query, "Fetching continuity blocks from indexer"
                );
                upper_attestation.block_number
            };

            match indexer
                .get_continuity_blocks(self.config.chain_key, query_block)
                .await
            {
                Ok(Some(attestation_with_proof)) => {
                    let indexer_blocks = attestation_with_proof.extract_blocks().map_err(|e| {
                        ContinuityError::Rpc(format!("Failed to extract blocks: {e}"))
                    })?;

                    if let Some(indexer_blocks) = indexer_blocks {
                        debug!(
                            indexer_blocks_count = indexer_blocks.len(),
                            first = indexer_blocks.first().map(|b| b.block_number).unwrap_or(0),
                            last = indexer_blocks.last().map(|b| b.block_number).unwrap_or(0),
                            "Found continuity blocks in indexer"
                        );

                        // If upper is predicted and indexer blocks don't extend to predicted upper bound,
                        // we need to build from source chain to extend
                        let needs_extension = is_predicted
                            && indexer_blocks
                                .last()
                                .map(|b| b.block_number < upper_attestation.block_number)
                                .unwrap_or(true);

                        if needs_extension {
                            info!(
                                indexer_last = indexer_blocks.last().map(|b| b.block_number),
                                predicted_upper = upper_attestation.block_number,
                                "Indexer blocks don't extend to predicted upper bound - building from source chain"
                            );
                            self.build_from_source_chain(
                                lower_attestation,
                                upper_attestation,
                                min_query,
                            )
                            .await?
                        } else {
                            // Process indexer blocks: combine, trim, append attestation
                            self.process_indexer_blocks(
                                indexer_blocks,
                                min_query,
                                &lower_attestation,
                                &upper_attestation,
                            )
                            .await?
                        }
                    } else {
                        info!("Indexer blocks not available - building from source chain");
                        self.build_from_source_chain(
                            lower_attestation,
                            upper_attestation,
                            min_query,
                        )
                        .await?
                    }
                }
                Ok(None) | Err(_) => {
                    info!("Indexer blocks not available - building from source chain");
                    self.build_from_source_chain(lower_attestation, upper_attestation, min_query)
                        .await?
                }
            }
        } else {
            // No indexer available, build from chain
            info!("No indexer configured - building from source chain");
            self.build_from_source_chain(lower_attestation, upper_attestation, min_query)
                .await?
        };

        // Create BuiltContinuityProof with the lower endpoint digest if available
        let proof = if let Some(lower_digest) = lower_endpoint_digest {
            BuiltContinuityProof::from_blocks_with_lower_digest(blocks, lower_digest)
        } else {
            BuiltContinuityProof::from_blocks(blocks)
        };

        Ok(proof)
    }

    pub async fn get_endpoints(
        &self,
        query_heights: &[u64],
        current_block: Option<u64>,
    ) -> ContinuityResult<(AttestationWithProof, AttestationWithProof)> {
        // Find the query range
        let min_query = *query_heights
            .iter()
            .min()
            .ok_or(ContinuityError::EmptyQuery)?;
        let max_query = *query_heights
            .iter()
            .max()
            .ok_or(ContinuityError::EmptyQuery)?;

        // Try to use indexer for attestation lookups if available
        use crate::builder::bounds::{BoundsFinder, Cc3BoundsFinder, IndexerBoundsFinder};

        let (lower, upper) = if let Some(ref indexer) = self.indexer_provider {
            let finder = IndexerBoundsFinder::new(self, indexer.as_ref());
            finder
                .find_bounds(min_query, max_query, current_block)
                .await?
        } else {
            let finder = Cc3BoundsFinder::new(self);
            finder
                .find_bounds(min_query, max_query, current_block)
                .await?
        };

        Ok((lower, upper))
    }

    /// Validate that a predicted upper bound block exists on the source chain.
    pub(crate) fn validate_predicted_upper_bound(
        &self,
        predicted_block: u64,
        query_block: u64,
        current_block: Option<u64>,
    ) -> ContinuityResult<()> {
        if let Some(current_block) = current_block {
            if predicted_block > current_block {
                return Err(ContinuityError::UpperBoundNotOnSourceChain {
                    query_block,
                    upper_block: predicted_block,
                    current_block,
                });
            }
        }
        Ok(())
    }

    /// Predict the next attestation block number based on the attestation interval.
    /// This is used for "eager" proof generation when a block is not yet attested.
    ///
    /// The formula aligns to the attestation interval boundary:
    /// `next_attestation = ((block / interval) + 1) * interval`
    pub(crate) async fn predict_next_attestation(
        &self,
        block: u64,
    ) -> ContinuityResult<AttestationWithProof> {
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

        // Return a predicted AttestationWithProof with a zero digest (will be computed from source chain blocks)
        // The digest field is not used for the upper bound in build_and_trim_continuity
        Ok(AttestationWithProof {
            block_number: next_attestation_block,
            root: H256::default(),
            digest: H256::default(),
            prev_digest: None,
            continuity_proof: None,
            continuity_proof_data: None,
        })
    }
}
