//! Continuity proof generation module
//!
//! This module handles fetching attestations from the Creditcoin3 chain
//! and building continuity proofs for query verification. It supports both
//! single queries and batch queries with a unified API.

use anyhow::{anyhow, Result};
use attestor_primitives::{block::Block, Query, SignedAttestation};
use cc_client::{AccountId32, Client as CcClient};
use ccnext_abi_encoding::abi::EncodingVersion;
use eth::{continuity::Manager as ContinuityManager, Client as EthClient};
use sp_core::H256;

/// Configuration for continuity proof generation
#[derive(Debug, Clone)]
pub struct ContinuityConfig {
    /// Creditcoin3 RPC URL
    pub cc3_rpc_url: String,
    /// Ethereum RPC URL for fetching blocks
    pub eth_rpc_url: String,
    /// Chain key for attestation lookup
    pub chain_key: u64,
}

/// Result of continuity proof generation
#[derive(Debug)]
pub struct ContinuityProof {
    /// The generated continuity blocks
    pub blocks: Vec<Block>,
}

/// Attestation boundaries for continuity proof
#[derive(Debug, Clone)]
pub struct AttestationBounds {
    /// Lower attestation (before or at query height)
    pub lower: Option<AttestationInfo>,
    /// Upper attestation (at or after query height)
    pub upper: Option<AttestationInfo>,
}

/// Information about an attestation
#[derive(Debug, Clone)]
pub struct AttestationInfo {
    pub block_number: u64,
    pub digest: H256,
}

/// Main continuity proof builder
pub struct ContinuityBuilder {
    config: ContinuityConfig,
    cc_client: CcClient,
    eth_client: EthClient,
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
    pub async fn build_for_single_query(&self, query: &Query) -> Result<ContinuityProof> {
        println!(
            "Building continuity proof for single query at height {}",
            query.height
        );

        let attestations = self.fetch_attestations().await?;
        if attestations.is_empty() {
            return Err(anyhow!(
                "No attestations found for chain_key {}. Queries require at least one attestation.",
                self.config.chain_key
            ));
        }

        let bounds = self.find_attestation_bounds(&[query.height], &attestations)?;
        let blocks = self.build_continuity_blocks(&bounds, query.height).await?;

        Ok(ContinuityProof { blocks })
    }

    /// Build continuity proof for multiple queries (batch)
    pub async fn build_for_batch_queries(&self, query_heights: &[u64]) -> Result<ContinuityProof> {
        if query_heights.is_empty() {
            return Err(anyhow!("No query heights provided"));
        }

        let min_height = *query_heights.iter().min().unwrap();
        let max_height = *query_heights.iter().max().unwrap();

        println!(
            "Building continuity proof for {} queries (range: {} to {})",
            query_heights.len(),
            min_height,
            max_height
        );

        let attestations = self.fetch_attestations().await?;
        if attestations.is_empty() {
            return Err(anyhow!(
                "No attestations found for chain_key {}. Queries require at least one attestation.",
                self.config.chain_key
            ));
        }

        let bounds = self.find_attestation_bounds(query_heights, &attestations)?;

        // For batch queries, we need continuity from the lower bound to the upper bound
        // This ensures all queries are covered by the same continuity chain
        let target_height = max_height;
        let blocks = self.build_continuity_blocks(&bounds, target_height).await?;

        Ok(ContinuityProof { blocks })
    }

    /// Fetch attestations from Creditcoin3
    async fn fetch_attestations(&self) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        match self
            .cc_client
            .get_attestations_for_chain(self.config.chain_key)
            .await
        {
            Ok(attestations) => {
                println!(
                    "Found {} attestations for chain_key {}",
                    attestations.len(),
                    self.config.chain_key
                );
                Ok(attestations)
            }
            Err(e) => {
                println!(
                    "Warning: Could not fetch attestations for chain_key {}: {:?}",
                    self.config.chain_key, e
                );
                Ok(vec![])
            }
        }
    }

    /// Find attestation bounds for the given query heights
    fn find_attestation_bounds(
        &self,
        query_heights: &[u64],
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<AttestationBounds> {
        let min_query = *query_heights.iter().min().unwrap();
        let max_query = *query_heights.iter().max().unwrap();

        // Find the latest attestation before or at the minimum query height
        let lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number <= min_query)
            .max_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            });

        // Find the earliest attestation at or after the maximum query height
        let upper = attestations
            .iter()
            .filter(|a| a.attestation.header_number >= max_query)
            .min_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            });

        if lower.is_none() {
            return Err(anyhow!(
                "No attestation found before or at block {}. The query block must come after at least one attestation.",
                min_query
            ));
        }

        // If no upper bound, we can still proceed with just the lower bound
        // The continuity chain will extend from the lower attestation to the query block

        println!(
            "Attestation bounds: lower={:?}, upper={:?}",
            lower.as_ref().map(|l| l.block_number),
            upper.as_ref().map(|u| u.block_number)
        );

        Ok(AttestationBounds { lower, upper })
    }

    /// Build the actual continuity blocks
    async fn build_continuity_blocks(
        &self,
        bounds: &AttestationBounds,
        target_height: u64,
    ) -> Result<Vec<Block>> {
        let lower_bound = bounds
            .lower
            .as_ref()
            .ok_or_else(|| anyhow!("Lower attestation bound is required"))?;

        // Determine the range of blocks to fetch
        // Start from the block AFTER the attestation
        let start_height = lower_bound.block_number + 1;
        let end_height = if let Some(upper) = &bounds.upper {
            upper.block_number.min(target_height)
        } else {
            target_height
        };

        // If start > end, we don't need any continuity blocks (query is at the attestation itself)
        if start_height > end_height {
            println!("Query is at attestation block, no continuity chain needed");
            return Ok(Vec::new());
        }

        println!("Building continuity chain from block {start_height} to {end_height}");

        // Use the eth::continuity::Manager to build the fragment
        let manager = ContinuityManager::new(start_height, end_height, &self.eth_client);

        // Use the digest from the lower bound attestation as the prev_digest
        // This links the continuity chain to the attestation
        let prev_digest = lower_bound.digest;

        // Create the attestation fragment using the manager
        let fragment = manager
            .create(prev_digest, EncodingVersion::V1)
            .await
            .map_err(|e| anyhow!("Failed to create continuity fragment: {}", e))?;

        // Extract the blocks from the fragment
        let continuity_blocks: Vec<Block> = fragment.blocks().to_vec();

        println!("Generated {} continuity blocks", continuity_blocks.len());

        Ok(continuity_blocks)
    }
}

// ===== Convenience functions for backward compatibility =====

/// Fetch continuity proof for a single query (legacy interface)
pub async fn fetch_continuity_proof(
    cc3_rpc_url: &str,
    query: &Query,
    eth_rpc_url: &str,
) -> Result<Vec<Block>> {
    let config = ContinuityConfig {
        cc3_rpc_url: cc3_rpc_url.to_string(),
        eth_rpc_url: eth_rpc_url.to_string(),
        chain_key: query.chain_id,
    };

    let builder = ContinuityBuilder::new(config).await?;
    let proof = builder.build_for_single_query(query).await?;

    Ok(proof.blocks)
}

/// Generate shared continuity for batch queries
pub async fn generate_shared_continuity(
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
