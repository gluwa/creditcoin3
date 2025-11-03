//! Continuity proof generation module
//!
//! This module handles fetching attestations and checkpoints from the Creditcoin3 chain
//! and building continuity proofs for query verification.

use anyhow::Result;
use attestor_primitives::Query;
use attestor_primitives::{block::Block, AttestationCheckpoint, SignedAttestation};
use cc_client::Client as CcClient;
use eth::{Client as EthClient, OrderedBlock};
use sp_core::H256;

// Type alias for continuity bounds to reduce complexity
type ContinuityBounds = (Option<(u64, H256)>, Option<(u64, H256)>);

/// Fetch continuity proof (attestations/checkpoints) for the given query
pub async fn fetch_continuity_proof(
    cc3_rpc_url: &str,
    query: &Query,
    block: &OrderedBlock,
    eth_rpc_url: &str,
) -> Result<Vec<Block>> {
    // Connect to Creditcoin3 chain to fetch attestations
    // Note: CcClient requires a key parameter, using empty string for read-only operations
    let cc_client = CcClient::new(cc3_rpc_url, "").await?;

    // Get attestations for the chain
    let chain_key = query.chain_id;

    // Try to get attestations and checkpoints, but handle the case where they don't exist yet
    let attestations = match cc_client.get_attestations_for_chain(chain_key).await {
        Ok(attestations) => attestations,
        Err(e) => {
            println!("Warning: Could not fetch attestations: {e:?}");
            vec![]
        }
    };

    let checkpoints = match cc_client.get_checkpoints_for_chain(chain_key).await {
        Ok(checkpoints) => checkpoints,
        Err(e) => {
            println!("Warning: Could not fetch checkpoints: {e:?}");
            vec![]
        }
    };

    if attestations.is_empty() && checkpoints.is_empty() {
        println!(
            "⚠️  No attestations or checkpoints found for chain_id {}.",
            query.chain_id
        );
        println!("   Queries require at least one attestation before the target block.");
        println!(
            "   Current block: {}, but no attestations exist yet.",
            query.height
        );
        return Ok(vec![]);
    }

    // Find the nearest attestations/checkpoints around the query height
    let (lower_bound, upper_bound) =
        find_continuity_bounds(query.height, &attestations[..], &checkpoints[..])?;

    // Build continuity fragment between the bounds
    let continuity_blocks =
        build_continuity_fragment(block, lower_bound, upper_bound, query.height, eth_rpc_url)
            .await?;

    println!(
        "Constructed continuity proof with {} blocks",
        continuity_blocks.len()
    );

    Ok(continuity_blocks)
}

/// Find the attestation/checkpoint bounds for the query height
fn find_continuity_bounds<A>(
    query_height: u64,
    attestations: &[SignedAttestation<sp_core::H256, A>],
    checkpoints: &[AttestationCheckpoint],
) -> Result<ContinuityBounds> {
    // Find highest attestation/checkpoint before query height
    let lower_attestation = attestations
        .iter()
        .filter(|a| a.attestation.header_number < query_height)
        .max_by_key(|a| a.attestation.header_number)
        .map(|a| (a.attestation.header_number, a.attestation.digest()));

    let lower_checkpoint = checkpoints
        .iter()
        .filter(|c| c.block_number < query_height)
        .max_by_key(|c| c.block_number)
        .map(|c| (c.block_number, c.digest));

    // Choose the higher of the two as lower bound
    let lower_bound = match (lower_attestation, lower_checkpoint) {
        (Some(a), Some(c)) => {
            if a.0 > c.0 {
                Some(a)
            } else {
                Some(c)
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    };

    // Find lowest attestation/checkpoint after query height
    let upper_attestation = attestations
        .iter()
        .filter(|a| a.attestation.header_number >= query_height)
        .min_by_key(|a| a.attestation.header_number)
        .map(|a| (a.attestation.header_number, a.attestation.digest()));

    let upper_checkpoint = checkpoints
        .iter()
        .filter(|c| c.block_number >= query_height)
        .min_by_key(|c| c.block_number)
        .map(|c| (c.block_number, c.digest));

    // Choose the lower of the two as upper bound
    let upper_bound = match (upper_attestation, upper_checkpoint) {
        (Some(a), Some(c)) => {
            if a.0 < c.0 {
                Some(a)
            } else {
                Some(c)
            }
        }
        (Some(a), None) => Some(a),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    };

    Ok((lower_bound, upper_bound))
}

/// Build continuity fragment as a chain of blocks
async fn build_continuity_fragment(
    block: &OrderedBlock,
    lower_bound: Option<(u64, H256)>,
    upper_bound: Option<(u64, H256)>,
    query_height: u64,
    eth_rpc_url: impl AsRef<str>,
) -> Result<Vec<Block>> {
    let mut continuity_blocks = Vec::new();

    // We need both bounds to create a valid continuity proof
    if let (Some((lower_height, lower_digest)), Some((upper_height, _upper_digest))) =
        (lower_bound, upper_bound)
    {
        // Check if the query block is within the bounds
        if query_height <= lower_height || query_height > upper_height {
            println!(
                "⚠️  Query height {query_height} is outside the attestation bounds [{lower_height}, {upper_height}]"
            );
            println!(
                "   The query block must be between attestations for continuity verification."
            );
            return Ok(vec![]);
        }

        // Build the continuity chain from lower_height + 1 to query_height
        // The chain starts from the block right after the lower attestation
        let start_height = lower_height + 1;

        // Create Ethereum client to fetch intermediate blocks
        let eth_client = EthClient::new(eth_rpc_url.as_ref(), None).await?;

        if start_height == query_height {
            // Query block is immediately after the attestation
            let query_block_root = eth::keccak_merkle_tree(block).root().to_h256();

            let continuity_block =
                Block::new_from_prev_digest(query_height, query_block_root, lower_digest);

            continuity_blocks.push(continuity_block);
            println!(
                "Built continuity proof with 1 block (query block immediately follows attestation)"
            );
        } else {
            // There are intermediate blocks between the attestation and query block
            println!("Fetching continuity chain from block {start_height} to {query_height}");

            // Build chain with actual block data
            let mut prev_digest = lower_digest;

            for height in start_height..=query_height {
                let block_to_process = if height == query_height {
                    // Use the already fetched query block
                    block.clone()
                } else {
                    // Fetch intermediate block
                    println!("Fetching intermediate block {height}...");
                    eth_client
                        .get_block(height, ccnext_abi_encoding::abi::EncodingVersion::V1)
                        .await?
                };

                let root = eth::keccak_merkle_tree(&block_to_process).root().to_h256();

                let continuity_block = Block::new_from_prev_digest(height, root, prev_digest);

                continuity_blocks.push(continuity_block.clone());
                prev_digest = continuity_block.digest;
            }

            println!(
                "Constructed continuity proof with {} blocks",
                continuity_blocks.len()
            );
        }
    } else if lower_bound.is_none() && upper_bound.is_none() {
        println!("No continuity bounds found - attestation system may not be initialized");
    } else {
        println!("Incomplete continuity bounds - need both lower and upper attestations");
    }

    Ok(continuity_blocks)
}
