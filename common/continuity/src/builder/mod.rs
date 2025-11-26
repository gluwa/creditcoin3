//! Continuity proof generation module
//!
//! This module handles fetching attestations from the Creditcoin3 chain
//! and building continuity proofs for query verification.
//!
//! Key concepts:
//! - Attestations: Consensus points on the Creditcoin3 chain that anchor block digests
//! - Continuity chains: Sequences of blocks that link a query block to attestations
//! - POC compliance: Chains must start at queryHeight-1 and end at next attestation

mod bounds;
mod build;
mod fetch;

pub use fetch::*;

use crate::{config::ContinuityConfig, proof::ContinuityProof};
use anyhow::{anyhow, Result};
use attestor_primitives::Query;
use cc_client::Client as CcClient;
use continuity_rpc::{SharedCcProvider, SharedEthProvider};
use eth::Client as EthClient;
use sp_core::H256;
use std::sync::Arc;

/// Continuity proof builder backed by abstract RPC providers.
pub struct ContinuityBuilder {
    pub(crate) config: ContinuityConfig,
    pub(crate) cc_provider: SharedCcProvider,
    pub(crate) eth_provider: SharedEthProvider,
}

impl ContinuityBuilder {
    /// Create a new builder with real RPC clients.
    pub async fn new(config: ContinuityConfig) -> Result<Self> {
        let cc_client = CcClient::new(&config.cc3_rpc_url, "")
            .await
            .map_err(|e| anyhow!("Failed to create CC client: {e}"))?;
        let eth_client = EthClient::new(&config.eth_rpc_url, None)
            .await
            .map_err(|e| anyhow!("Failed to create ETH client: {e}"))?;
        Ok(Self::new_with_providers(
            config,
            Arc::new(cc_client),
            Arc::new(eth_client),
        ))
    }

    /// Create a builder using injected (possibly mocked) providers.
    pub fn new_with_providers(
        config: ContinuityConfig,
        cc_provider: SharedCcProvider,
        eth_provider: SharedEthProvider,
    ) -> Self {
        Self {
            config,
            cc_provider,
            eth_provider,
        }
    }

    /// Build continuity proof for a single query
    ///
    /// Fetches attestations and builds the minimal continuity chain needed
    /// to verify the query. The chain starts at queryHeight-1 and extends
    /// to the next attestation/checkpoint after the query.
    pub async fn build_for_single_query(&self, query: &Query) -> Result<ContinuityProof> {
        println!(
            "Building continuity proof for single query at height {}",
            query.height
        );
        self.build_for_heights(&[query.height]).await
    }
    /// Build continuity proof for multiple queries (batch)
    ///
    /// Optimizes gas usage by building a single continuity chain that covers
    /// all query heights. The chain spans from min(queryHeights)-1 to the
    /// next attestation after max(queryHeights).
    pub async fn build_for_batch_queries(&self, query_heights: &[u64]) -> Result<ContinuityProof> {
        if query_heights.is_empty() {
            return Err(anyhow!("No query heights provided"));
        }

        let (min, max) = (
            *query_heights.iter().min().unwrap(),
            *query_heights.iter().max().unwrap(),
        );
        println!(
            "Building continuity proof for {} queries (range: {} to {})",
            query_heights.len(),
            min,
            max
        );

        self.build_for_heights(query_heights).await
    }

    /// Helper to fetch raw transaction bytes for a block using the underlying Eth provider.
    pub async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        self.eth_provider.get_block_tx_bytes(block_number).await
    }
    /// Resolve a transaction hash to its block number and index on the source chain.
    pub async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<(u64, u64)> {
        self.eth_provider.get_tx_position_by_hash(tx_hash).await
    }
}
