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

use crate::config::ContinuityConfig;
use crate::proof::ContinuityProof;

use attestor_primitives::Query;
use cc_client::Client as CcClient;
use eth::Client as EthClient;

use anyhow::{anyhow, Result};

/// Main continuity proof builder.
pub struct ContinuityBuilder {
    pub config: ContinuityConfig,
    pub cc_client: CcClient,
    pub eth_client: EthClient,
}

impl ContinuityBuilder {
    /// Create a new continuity builder
    pub async fn new(config: ContinuityConfig) -> Result<Self> {
        let cc_client = CcClient::new(&config.cc3_rpc_url, "")
            .await
            .map_err(|e| anyhow!("Failed to create CC client: {}", e))?;

        let eth_client = EthClient::new(&config.eth_rpc_url, None)
            .await
            .map_err(|e| anyhow!("Failed to create ETH client: {}", e))?;

        Ok(Self {
            config,
            cc_client,
            eth_client,
        })
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
}
