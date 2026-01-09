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

pub use build::ContinuityResult;
pub use fetch::*;

use crate::{
    config::ContinuityConfig,
    errors::ContinuityError,
    proof::ContinuityProof,
    rpc::{SharedCcProvider, SharedEthProvider},
    AttestationInfo,
};
use anyhow::{Context, Result};
use cc_client::Client as CcClient;
use eth::Client as EthClient;
use sp_core::H256;
use std::sync::Arc;
use tracing::info;

/// This enum wraps a bool to provide clarity in return values
/// A continuity proof is considered to `EndInAttestation` if
/// an attestation was used as the upper endpoint when constructing
/// it.
#[derive(Debug, Clone)]
pub enum EndsInAttestation {
    True,
    False,
}

impl From<EndsInAttestation> for bool {
    fn from(e: EndsInAttestation) -> bool {
        matches!(e, EndsInAttestation::True)
    }
}

/// Continuity proof builder backed by abstract RPC providers.
pub struct ContinuityBuilder {
    pub config: ContinuityConfig,
    pub(crate) cc_provider: SharedCcProvider,
    pub(crate) eth_provider: SharedEthProvider,
}

impl ContinuityBuilder {
    /// Create a new builder with real RPC clients.
    /// Note: Uses read-only CC client since signing is not needed for proof generation.
    pub async fn new(config: ContinuityConfig) -> Result<Self> {
        let cc_client = CcClient::new_read_only(&config.cc3_rpc_url)
            .await
            .context("Failed to create CC client")?;
        let eth_client = EthClient::new(&config.eth_rpc_url, None)
            .await
            .context("Failed to create ETH client")?;

        Ok(Self::new_with_providers(
            config,
            Arc::new(cc_client),
            Arc::new(eth_client),
        ))
    }

    #[cfg(feature = "block_cache")]
    pub async fn new_with_block_caching(
        config: ContinuityConfig,
        cache_config: eth::block_cache::BlockCacheConfig,
    ) -> Result<Self> {
        let cc_client = CcClient::new_read_only(&config.cc3_rpc_url)
            .await
            .context("Failed to create CC client")?;

        let eth_client = EthClient::new_with_cache(&config.eth_rpc_url, None, cache_config)
            .await
            .context("Failed to create caching ETH client")?;

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
    pub async fn build_for_single_query(
        &self,
        height: u64,
        lower_attestation: AttestationInfo,
        upper_attestation: AttestationInfo,
    ) -> ContinuityResult<ContinuityProof> {
        info!(
            query_height = height,
            "Building continuity proof for single query"
        );
        self.build_for_heights(&[height], lower_attestation, upper_attestation)
            .await
    }

    /// Build continuity proof for multiple queries (batch)
    ///
    /// Optimizes gas usage by building a single continuity chain that covers
    /// all query heights. The chain spans from min(queryHeights)-1 to the
    /// next attestation after max(queryHeights).
    pub async fn build_for_batch_queries(
        &self,
        query_heights: &[u64],
        lower_attestation: AttestationInfo,
        upper_attestation: AttestationInfo,
    ) -> ContinuityResult<ContinuityProof> {
        if query_heights.is_empty() {
            return Err(ContinuityError::EmptyQuery);
        }

        let (min, max) = (
            *query_heights.iter().min().unwrap(),
            *query_heights.iter().max().unwrap(),
        );
        info!(
            query_count = query_heights.len(),
            min_height = min,
            max_height = max,
            "Building continuity proof for batch queries"
        );

        self.build_for_heights(query_heights, lower_attestation, upper_attestation)
            .await
    }

    /// Helper to fetch raw transaction bytes for a block using the underlying Eth provider.
    pub async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        self.eth_provider.get_block_tx_bytes(block_number).await
    }

    /// Get the transaction hash at a specific index in a block.
    pub async fn get_tx_hash_by_index(
        &self,
        block_number: u64,
        tx_index: u64,
    ) -> Result<Option<H256>> {
        self.eth_provider
            .get_tx_hash_by_index(block_number, tx_index)
            .await
    }

    /// Resolve a transaction hash to its block number and index on the source chain.
    pub async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<(u64, u64)> {
        self.eth_provider.get_tx_position_by_hash(tx_hash).await
    }

    /// Get the current block height (latest block number).
    pub async fn get_last_block(&self) -> Result<u64> {
        self.eth_provider.get_last_block().await
    }

    /// Get the CC3 chain name for health check purposes.
    pub async fn get_chain_name(&self) -> Result<String> {
        self.cc_provider
            .get_chain_name()
            .await
            .context("Failed to get CC3 chain name")
    }

    /// Get the ETH chain ID for health check purposes.
    pub async fn get_eth_chain_id(&self) -> Result<u64> {
        self.eth_provider
            .get_chain_id()
            .await
            .context("Failed to get ETH chain ID")
    }

    /// Get the attestation genesis block number for the configured chain.
    /// This is the first block that can be attested to.
    pub async fn get_attestation_genesis_block(&self) -> Result<u64> {
        self.cc_provider
            .get_attestation_chain_genesis_block_number(self.config.chain_key)
            .await
            .context("Failed to get attestation genesis block number")
    }

    /// Get the last attested block number for the configured chain.
    /// Returns `None` if no attestations exist yet, otherwise returns the highest
    /// attestation header_number.
    /// Uses lightweight queries: fetch_last_digest + get_attestation_by_digest
    pub async fn get_last_attested_block(&self) -> Result<Option<u64>> {
        // First, get the last digest (lightweight query)
        let last_digest = self
            .cc_provider
            .fetch_last_digest(self.config.chain_key)
            .await?;

        let Some(digest) = last_digest else {
            return Ok(None); // No attestations yet
        };

        // Then fetch the attestation by digest to get the header_number
        let attestation = self
            .cc_provider
            .get_attestation_by_digest(self.config.chain_key, digest)
            .await?;

        Ok(attestation.map(|a| a.attestation.header_number))
    }
}
