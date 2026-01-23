//! CC3 chain data fetching methods
//!
//! This module contains methods that fetch data directly from the Creditcoin3 chain,
//! as opposed to using the indexer (which is handled in build.rs).

use super::ContinuityBuilder;
use crate::config::ContinuityConfig;
use anyhow::{Context, Result};
use attestor_primitives::{block::Block, AttestationCheckpoint, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;

impl ContinuityBuilder {
    /// Fetch attestations from Creditcoin3 chain.
    /// This is used when no indexer is available or as a fallback.
    pub async fn fetch_attestations(&self) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.cc_provider
            .get_attestations_for_chain(self.config.chain_key)
            .await
            .context("Failed to fetch attestations from CC3 chain")
    }

    /// Check if query is at a checkpoint height by checking for a checkpoint at the specific height
    ///
    /// When an indexer is available, it will be used instead of querying the chain directly.
    pub async fn check_if_at_checkpoint_height(
        &self,
        query_height: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        // If indexer is available, use it for checkpoint queries
        if let Some(ref indexer) = self.indexer_provider {
            if let Ok(checkpoint) = indexer
                .get_checkpoint_by_height(self.config.chain_key, query_height)
                .await
            {
                return Ok(checkpoint);
            }
            // If indexer fails, fall through to chain query
        }

        // Fallback to chain query when indexer is not available or fails
        self.cc_provider
            .get_checkpoint_by_height(self.config.chain_key, query_height)
            .await
            .context("Failed to fetch checkpoint at query height")
    }

    /// Fetch checkpoints with optional filtering
    ///
    /// Returns checkpoints sorted highest to lowest (newest first).
    ///
    /// When an indexer is available, it will be used instead of querying the chain directly.
    pub async fn fetch_checkpoints_smart(
        &self,
        max_needed: Option<u64>,
        min_needed: Option<u64>,
    ) -> Result<Vec<AttestationCheckpoint>> {
        // If indexer is available, use it for checkpoint queries
        if let Some(ref indexer) = self.indexer_provider {
            // OPTIMIZATION: If we only need to check the last checkpoint, use the optimized query
            if max_needed.is_none() && min_needed.is_none() {
                if let Ok(Some(last_checkpoint)) =
                    indexer.get_last_checkpoint(self.config.chain_key).await
                {
                    return Ok(vec![last_checkpoint]);
                }
            }

            // OPTIMIZATION: If we have a query height (min_needed), fetch checkpoints around it
            // instead of fetching all checkpoints
            // Use checkpoint_block_interval * 10 to handle cases where historical
            // checkpoint intervals were much larger than the current interval.
            if let Some(query_height) = min_needed {
                let max_range = self.config.checkpoint_block_interval() * 10;
                if let Ok(checkpoints) = indexer
                    .get_checkpoints_around_height(self.config.chain_key, query_height, max_range)
                    .await
                {
                    // Filter checkpoints based on block number range if max_needed is specified
                    let filtered: Vec<AttestationCheckpoint> = checkpoints
                        .into_iter()
                        .filter(|c| {
                            if let Some(max) = max_needed {
                                if c.block_number >= max {
                                    return false;
                                }
                            }
                            true
                        })
                        .collect();

                    return Ok(filtered);
                }
            }

            // Fallback: Fetch all checkpoints from indexer (already sorted DESC by block number)
            // This should rarely be needed now
            if let Ok(all_checkpoints) = indexer
                .get_checkpoints_for_chain(self.config.chain_key)
                .await
            {
                // Filter checkpoints based on block number range
                let mut filtered: Vec<AttestationCheckpoint> = all_checkpoints
                    .into_iter()
                    .filter(|c| {
                        max_needed.is_none_or(|max| c.block_number < max)
                            && min_needed.is_none_or(|min| c.block_number > min)
                    })
                    .collect();

                // Ensure sorted (should already be sorted from indexer, but double-check)
                filtered.sort_by_key(|c| std::cmp::Reverse(c.block_number));

                return Ok(filtered);
            }
            // If indexer fails, fall through to chain query
        }

        // Fallback to chain query when indexer is not available or fails
        // Start with last checkpoint (most efficient single query)
        let last_checkpoint = self
            .cc_provider
            .get_last_checkpoint(self.config.chain_key)
            .await
            .context("Failed to fetch last checkpoint")?;

        // If we only need to check the last checkpoint, we're done
        if max_needed.is_none() && min_needed.is_none() {
            return Ok(last_checkpoint.into_iter().collect());
        }

        // Fetch all checkpoints from chain
        let all_checkpoints = self
            .cc_provider
            .get_checkpoints_for_chain(self.config.chain_key)
            .await
            .context("Failed to fetch checkpoints")?;

        // Filter checkpoints based on block number range
        let mut filtered: Vec<AttestationCheckpoint> = all_checkpoints
            .into_iter()
            .filter(|c| {
                max_needed.is_none_or(|max| c.block_number < max)
                    && min_needed.is_none_or(|min| c.block_number > min)
            })
            .collect();

        // CRITICAL: Sort checkpoints by block number (descending - newest first)
        // This ensures we can correctly find the next checkpoint after a query
        filtered.sort_by_key(|c| std::cmp::Reverse(c.block_number));

        Ok(filtered)
    }
}

// ===== Convenience functions for backward compatibility =====

/// Fetch continuity proof for a single query (legacy interface)
///
/// # Note
///
/// This is a legacy convenience function. For new code, prefer using
/// [`ContinuityBuilder`] directly for better control and performance.
pub async fn fetch_continuity_proof(
    cc3_rpc_url: &str,
    eth_rpc_url: &str,
    chain_key: u64,
    height: u64,
) -> Result<Vec<Block>> {
    let config = ContinuityConfig::builder()
        .cc3_rpc_url(cc3_rpc_url)
        .eth_rpc_url(eth_rpc_url)
        .chain_key(chain_key)
        .fetch_intervals()
        .await?;

    let builder = ContinuityBuilder::new(config).await?;
    let (lower_attestation, upper_attestation) = builder.get_endpoints(&[height], None).await?;
    let proof = builder
        .build_for_single_query(height, lower_attestation, upper_attestation)
        .await?;

    Ok(proof.blocks)
}

/// Fetch continuity proof for multiple queries (legacy interface)
///
/// # Note
///
/// This is a legacy convenience function. For new code, prefer using
/// [`ContinuityBuilder`] directly for better control and performance.
pub async fn fetch_continuity_proof_batch(
    cc3_rpc_url: &str,
    eth_rpc_url: &str,
    chain_key: u64,
    query_heights: &[u64],
) -> Result<Vec<Block>> {
    let config = ContinuityConfig::builder()
        .cc3_rpc_url(cc3_rpc_url)
        .eth_rpc_url(eth_rpc_url)
        .chain_key(chain_key)
        .fetch_intervals()
        .await?;

    let builder = ContinuityBuilder::new(config).await?;

    let (lower_attestation, upper_attestation) = builder.get_endpoints(query_heights, None).await?;
    let proof = builder
        .build_for_batch_queries(query_heights, lower_attestation, upper_attestation)
        .await?;

    Ok(proof.blocks)
}
