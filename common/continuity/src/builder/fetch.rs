use super::ContinuityBuilder;
use crate::config::ContinuityConfig;
use anyhow::{anyhow, Result};
use attestor_primitives::{block::Block, AttestationCheckpoint, Query, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;

impl ContinuityBuilder {
    /// Fetch attestations for this chain.
    pub async fn fetch_attestations(&self) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.cc_client
            .get_attestations_for_chain(self.config.chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch attestations: {}", e))
    }

    /// Check whether a query height is itself a checkpoint height.
    pub async fn check_if_at_checkpoint_height(
        &self,
        query_height: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        let last_checkpoint = self
            .cc_client
            .get_last_checkpoint(self.config.chain_key)
            .await?;

        if let Some(cp) = last_checkpoint {
            if cp.block_number == query_height {
                return Ok(Some(cp));
            }
        }

        Ok(None)
    }

    /// Fetch checkpoints and optionally filter by range.
    pub async fn fetch_checkpoints_smart(
        &self,
        max_needed: Option<u64>,
        min_needed: Option<u64>,
    ) -> Result<Vec<AttestationCheckpoint>> {
        let last_cp = self
            .cc_client
            .get_last_checkpoint(self.config.chain_key)
            .await?;

        if max_needed.is_none() && min_needed.is_none() {
            return Ok(last_cp.into_iter().collect());
        }

        let all = self
            .cc_client
            .get_checkpoints_for_chain(self.config.chain_key)
            .await?;

        Ok(all
            .into_iter()
            .filter(|c| {
                if let Some(maxv) = max_needed {
                    if c.block_number >= maxv {
                        return false;
                    }
                }
                if let Some(minv) = min_needed {
                    if c.block_number <= minv {
                        return false;
                    }
                }
                true
            })
            .collect())
    }
}

/// Fetch continuity proof for a single query (legacy interface)
pub async fn fetch_continuity_proof(
    cc3_rpc_url: &str,
    eth_rpc_url: &str,
    chain_key: u64,
    query: &Query,
) -> Result<Vec<Block>> {
    let config = ContinuityConfig {
        cc3_rpc_url: cc3_rpc_url.to_string(),
        eth_rpc_url: eth_rpc_url.to_string(),
        chain_key,
    };

    let builder = ContinuityBuilder::new(config).await?;
    let proof = builder.build_for_single_query(query).await?;

    Ok(proof.blocks)
}

/// Fetch continuity proof for multiple queries (legacy interface)
pub async fn fetch_continuity_proof_batch(
    cc3_rpc_url: &str,
    eth_rpc_url: &str,
    chain_key: u64,
    query_heights: &[u64],
) -> Result<Vec<Block>> {
    let config = ContinuityConfig {
        cc3_rpc_url: cc3_rpc_url.to_string(),
        eth_rpc_url: eth_rpc_url.to_string(),
        chain_key,
    };

    let builder = ContinuityBuilder::new(config).await?;
    let proof = builder.build_for_batch_queries(query_heights).await?;

    Ok(proof.blocks)
}
