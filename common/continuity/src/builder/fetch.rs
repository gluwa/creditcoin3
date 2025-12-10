use super::ContinuityBuilder;
use crate::config::ContinuityConfig;
use anyhow::{Context, Result};
use attestor_primitives::{block::Block, AttestationCheckpoint, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;

impl ContinuityBuilder {
    /// Fetch attestations from Creditcoin3
    pub async fn fetch_attestations(&self) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.cc_provider
            .get_attestations_for_chain(self.config.chain_key)
            .await
            .context("Failed to fetch attestations")
    }

    /// Check if query is at a checkpoint height by checking the last checkpoint
    pub async fn check_if_at_checkpoint_height(
        &self,
        query_height: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        let last_checkpoint = self
            .cc_provider
            .get_last_checkpoint(self.config.chain_key)
            .await
            .context("Failed to fetch last checkpoint")?;

        if let Some(checkpoint) = last_checkpoint {
            if checkpoint.block_number == query_height {
                return Ok(Some(checkpoint));
            }
        }

        Ok(None)
    }

    /// Fetch checkpoints with optional filtering
    ///
    /// Since iteration order is not guaranteed, we fetch all checkpoints
    /// and then filter them. Returns checkpoints sorted highest to lowest (newest first).
    pub async fn fetch_checkpoints_smart(
        &self,
        max_needed: Option<u64>,
        min_needed: Option<u64>,
    ) -> Result<Vec<AttestationCheckpoint>> {
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

        // Fetch all checkpoints (iteration order is not guaranteed, so we need all)
        let all_checkpoints = self
            .cc_provider
            .get_checkpoints_for_chain(self.config.chain_key)
            .await
            .context("Failed to fetch checkpoints")?;

        // Filter checkpoints based on block number range
        let filtered: Vec<AttestationCheckpoint> = all_checkpoints
            .into_iter()
            .filter(|c| {
                if let Some(max) = max_needed {
                    if c.block_number >= max {
                        return false;
                    }
                }
                if let Some(min) = min_needed {
                    if c.block_number <= min {
                        return false;
                    }
                }
                true
            })
            .collect();

        Ok(filtered)
    }
}

// ===== Convenience functions for backward compatibility =====

/// Fetch continuity proof for a single query (legacy interface)
pub async fn fetch_continuity_proof(
    cc3_rpc_url: &str,
    cc3_key: &str,
    eth_rpc_url: &str,
    chain_key: u64,
    height: u64,
) -> Result<Vec<Block>> {
    let config = ContinuityConfig {
        cc3_rpc_url: cc3_rpc_url.to_string(),
        cc3_key: cc3_key.to_string(),
        eth_rpc_url: eth_rpc_url.to_string(),
        chain_key,
    };

    let builder = ContinuityBuilder::new(config).await?;
    let (proof, _) = builder.build_for_single_query(height).await?;

    Ok(proof.blocks)
}

/// Fetch continuity proof for multiple queries (legacy interface)
pub async fn fetch_continuity_proof_batch(
    cc3_rpc_url: &str,
    cc3_key: &str,
    eth_rpc_url: &str,
    chain_key: u64,
    query_heights: &[u64],
) -> Result<Vec<Block>> {
    let config = ContinuityConfig {
        cc3_rpc_url: cc3_rpc_url.to_string(),
        cc3_key: cc3_key.to_string(),
        eth_rpc_url: eth_rpc_url.to_string(),
        chain_key,
    };

    let builder = ContinuityBuilder::new(config).await?;
    let (proof, _) = builder.build_for_batch_queries(query_heights).await?;

    Ok(proof.blocks)
}
