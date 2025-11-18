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
use attestor_primitives::{block::Block, AttestationCheckpoint, Query, SignedAttestation};
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

impl ContinuityProof {
    /// Create from Vec<Block>
    pub fn from_blocks(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }
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
        // Fetch attestations (always needed)
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

        // Find attestation bounds (handles queries at attestation/checkpoint heights)
        // Checkpoints are fetched lazily only when needed
        let (lower, upper) = self
            .find_attestation_bounds(min_query, max_query, &attestations)
            .await?;

        // Build and trim continuity blocks
        let blocks = self
            .build_and_trim_continuity(lower, upper, min_query)
            .await?;

        Ok(ContinuityProof::from_blocks(blocks))
    }

    /// Fetch attestations from Creditcoin3
    async fn fetch_attestations(&self) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.cc_client
            .get_attestations_for_chain(self.config.chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch attestations: {}", e))
    }

    /// Check if query is at a checkpoint height by checking the last checkpoint
    async fn check_if_at_checkpoint_height(
        &self,
        query_height: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        let last_checkpoint = self
            .cc_client
            .get_last_checkpoint(self.config.chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch last checkpoint: {}", e))?;

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
    async fn fetch_checkpoints_smart(
        &self,
        max_needed: Option<u64>,
        min_needed: Option<u64>,
    ) -> Result<Vec<AttestationCheckpoint>> {
        // Start with last checkpoint (most efficient single query)
        let last_checkpoint = self
            .cc_client
            .get_last_checkpoint(self.config.chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch last checkpoint: {}", e))?;

        // If we only need to check the last checkpoint, we're done
        if max_needed.is_none() && min_needed.is_none() {
            return Ok(last_checkpoint.into_iter().collect());
        }

        // Fetch all checkpoints (iteration order is not guaranteed, so we need all)
        let all_checkpoints = self
            .cc_client
            .get_checkpoints_for_chain(self.config.chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch checkpoints: {}", e))?;

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

    /// Find optimal attestation bounds for the query range
    ///
    /// Handles special case: when query is at an attestation or checkpoint height,
    /// we need to fetch the previous attestation/checkpoint to compute the continuity proof.
    async fn find_attestation_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<(AttestationInfo, Option<AttestationInfo>)> {
        // Check if query is exactly at an attestation height
        let is_at_attestation = attestations
            .iter()
            .any(|a| a.attestation.header_number == min_query);

        // Check if query is at checkpoint height (lazy check - only queries last checkpoint)
        let is_at_checkpoint = self
            .check_if_at_checkpoint_height(min_query)
            .await?
            .is_some();

        if is_at_attestation || is_at_checkpoint {
            println!(
                "Query is at {} height (attestation: {}, checkpoint: {})",
                if is_at_attestation {
                    "attestation"
                } else {
                    "checkpoint"
                },
                is_at_attestation,
                is_at_checkpoint
            );
            println!("Fetching previous attestation/checkpoint to build continuity proof...");
        }

        // Find lower bound: closest attestation or checkpoint before min_query
        // Requires continuity to start at queryHeight - 1, so we need consensus point before that
        // If query is at an attestation/checkpoint height, we need the previous one
        let required_before = min_query.saturating_sub(1);

        // Find best lower bound from attestations
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number < required_before)
            .max_by_key(|a| a.attestation.header_number);

        // Only fetch checkpoints if:
        // 1. Query is at checkpoint height (need previous checkpoint)
        // 2. No attestation found before required_before (need to check checkpoints)
        let checkpoint_lower = if is_at_checkpoint || attestation_lower.is_none() {
            // Use max_needed to get checkpoints BEFORE required_before (not min_needed which filters them out!)
            let checkpoints = self
                .fetch_checkpoints_smart(Some(required_before), None)
                .await?;

            checkpoints
                .into_iter()
                .filter(|c| c.block_number < required_before)
                .max_by_key(|c| c.block_number)
        } else {
            None
        };

        // Choose the closest one (highest block number) before required_before
        let lower_info = match (attestation_lower, checkpoint_lower) {
            (Some(a), Some(c)) => {
                if a.attestation.header_number > c.block_number {
                    AttestationInfo {
                        block_number: a.attestation.header_number,
                        digest: a.attestation.digest(),
                    }
                } else {
                    AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
                    }
                }
            }
            (Some(a), None) => AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            },
            (None, Some(c)) => AttestationInfo {
                block_number: c.block_number,
                digest: c.digest,
            },
            (None, None) => {
                // Provide helpful error message with suggestions
                let error_msg = format!(
                    "No attestation or checkpoint found before block {required_before} (query height: {min_query}).\n\
                    The continuity proof requires a consensus point (attestation or checkpoint) \
                    before block {required_before} to start the continuity chain.\n\n\
                    Possible solutions:\n\
                    1. Ensure checkpoints are imported for chain_key {} using import_checkpoints\n\
                    2. Wait for an attestation at a block before the query height\n\
                    3. Query a block height that has an earlier attestation/checkpoint",
                    self.config.chain_key
                );
                return Err(anyhow!(error_msg));
            }
        };

        // Find upper bound: if query is at an attestation/checkpoint height, use that as upper bound
        // Otherwise, find the next attestation/checkpoint after max_query
        let upper_info = if is_at_attestation {
            // Query is at an attestation height - use that attestation as upper bound
            attestations
                .iter()
                .find(|a| a.attestation.header_number == max_query)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                })
        } else if is_at_checkpoint {
            // Query is at a checkpoint height - use that checkpoint as upper bound
            let checkpoints = self.fetch_checkpoints_smart(None, None).await?;
            checkpoints
                .into_iter()
                .find(|c| c.block_number == max_query)
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                })
        } else {
            // Query is not at an attestation/checkpoint height - find next one after max_query
            let attestation_upper = attestations
                .iter()
                .filter(|a| a.attestation.header_number > max_query)
                .min_by_key(|a| a.attestation.header_number)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                });

            // Only fetch checkpoints for upper bound if no attestation found after max_query
            let checkpoint_upper = if attestation_upper.is_none() {
                let checkpoints = self.fetch_checkpoints_smart(Some(max_query), None).await?;
                checkpoints
                    .into_iter()
                    .filter(|c| c.block_number > max_query)
                    .min_by_key(|c| c.block_number)
                    .map(|c| AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
                    })
            } else {
                None
            };

            // Choose the closest one (lowest block number) after max_query
            match (attestation_upper, checkpoint_upper) {
                (Some(a), Some(c)) => {
                    if a.block_number < c.block_number {
                        Some(a)
                    } else {
                        Some(c)
                    }
                }
                (Some(a), None) => Some(a),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            }
        };

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
