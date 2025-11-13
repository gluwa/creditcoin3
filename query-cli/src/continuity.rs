//! Continuity proof generation module
//!
//! This module handles fetching attestations from the Creditcoin3 chain
//! and building continuity proofs for query verification.
//!
//! Key concepts:
//! - Attestations: Consensus points on the Creditcoin3 chain that anchor block digests
//! - Continuity chains: Sequences of blocks that link a query block to attestations
//! - POC compliance: Chains must start at queryHeight-1 and end at next attestation

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

/// Information about an attestation
#[derive(Debug, Clone)]
struct AttestationInfo {
    block_number: u64,
    digest: H256,
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

    /// Core logic for building continuity proof for given heights
    async fn build_for_heights(&self, query_heights: &[u64]) -> Result<ContinuityProof> {
        // Fetch attestations once
        let attestations = self.fetch_attestations().await?;
        if attestations.is_empty() {
            return Err(anyhow!(
                "No attestations found for chain_key {}. Queries require at least one attestation.",
                self.config.chain_key
            ));
        }

        // Find the query range
        let min_query = *query_heights.iter().min().unwrap();
        let max_query = *query_heights.iter().max().unwrap();

        // Find attestation bounds
        let (lower, upper) = self.find_attestation_bounds(min_query, max_query, &attestations)?;

        // Build and trim continuity blocks
        let blocks = self
            .build_and_trim_continuity(lower, upper, min_query)
            .await?;

        Ok(ContinuityProof { blocks })
    }

    /// Fetch attestations from Creditcoin3
    async fn fetch_attestations(&self) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.cc_client
            .get_attestations_for_chain(self.config.chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch attestations: {}", e))
    }

    /// Find optimal attestation bounds for the query range
    fn find_attestation_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<(AttestationInfo, Option<AttestationInfo>)> {
        // Find lower bound: closest attestation before min_query
        // POC requires continuity to start at queryHeight - 1, so we need attestation before that
        let required_before = min_query.saturating_sub(1);

        let lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number < required_before)
            .max_by_key(|a| a.attestation.header_number)
            .ok_or_else(|| {
                anyhow!(
                    "No attestation found before block {}. Need an earlier attestation to build continuity.",
                    required_before
                )
            })?;

        let lower_info = AttestationInfo {
            block_number: lower.attestation.header_number,
            digest: lower.attestation.digest(),
        };

        // Find upper bound: closest attestation after max_query
        let upper_info = attestations
            .iter()
            .filter(|a| a.attestation.header_number > max_query)
            .min_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            });

        // Log the bounds for debugging
        println!(
            "Attestation bounds: lower={} upper={}",
            lower_info.block_number,
            upper_info
                .as_ref()
                .map(|u| u.block_number)
                .unwrap_or(max_query + 10)
        );

        Ok((lower_info, upper_info))
    }

    /// Build continuity blocks and trim to required range
    async fn build_and_trim_continuity(
        &self,
        lower: AttestationInfo,
        upper: Option<AttestationInfo>,
        min_query: u64,
    ) -> Result<Vec<Block>> {
        // POC pattern: continuity chain ALWAYS starts at queryHeight - 1
        let required_start = min_query.saturating_sub(1);

        // Determine end height (next consensus point or fallback)
        let end_height = upper
            .map(|u| u.block_number)
            .unwrap_or_else(|| min_query + 10);

        // Build from attestation to end to get correct digests
        let build_start = lower.block_number + 1;

        println!(
            "Building continuity chain from {build_start} to {end_height} (will trim to start at {required_start})"
        );

        // Create continuity fragment
        let manager = ContinuityManager::new(build_start, end_height, &self.eth_client);
        let fragment = manager
            .create(lower.digest, EncodingVersion::V1)
            .await
            .map_err(|e| anyhow!("Failed to create continuity fragment: {}", e))?;

        let all_blocks: Vec<Block> = fragment.blocks().to_vec();

        // If we built from the required start, no trimming needed
        if build_start == required_start {
            println!("Generated {} continuity blocks", all_blocks.len());
            return Ok(all_blocks);
        }

        // Trim to start at required_start
        let start_index = all_blocks
            .iter()
            .position(|b| b.block_number == required_start)
            .ok_or_else(|| {
                anyhow!(
                    "Block {} not found in continuity chain (range: {}-{})",
                    required_start,
                    all_blocks.first().map(|b| b.block_number).unwrap_or(0),
                    all_blocks.last().map(|b| b.block_number).unwrap_or(0)
                )
            })?;

        let trimmed = all_blocks[start_index..].to_vec();

        println!(
            "Trimmed continuity chain from {} to {} blocks (starting at block {})",
            all_blocks.len(),
            trimmed.len(),
            required_start
        );

        Ok(trimmed)
    }
}

// ===== Convenience functions for backward compatibility =====

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
