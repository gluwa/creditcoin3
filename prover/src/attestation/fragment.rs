use std::str::FromStr;

use anyhow::Result;
use attestation_chain::block::Block;
use hex::ToHex;
use sp_core::H256;
use starknet_types_core::felt::Felt;
use thiserror::Error;
use tracing::debug;

use attestation_chain::attestation_fragment::{AttestationFragment, AttestationFragmentError};
use attestation_chain::continuity_chain::{
    Error as FragmentManagerError, Manager as FragmentManager,
};
use eth::Client;
use pallet_prover_primitives::Query;

use crate::attestation::create_block_with_prev_digest;
use crate::postgres::blockwithdigest::BlockWithDigest;
use crate::postgres::from_storage_type;
use crate::{postgres, AttestationCacheType};

#[derive(Debug, Clone, PartialEq)]
pub enum FragmentType {
    /// All fragments ending in a checkpoint also start with a checkpoint
    CheckpointOnEachEnd,
    /// Most fragments ending in an attestation also start with an attestation
    AttestationOnEachEnd,
}

impl std::fmt::Display for FragmentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FragmentType::CheckpointOnEachEnd => write!(f, "CheckpointOnEachEnd"),
            FragmentType::AttestationOnEachEnd => write!(f, "AttestationOnEachEnd"),
        }
    }
}

impl FromStr for FragmentType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "CheckpointOnEachEnd" => Ok(FragmentType::CheckpointOnEachEnd),
            "AttestationOnEachEnd" => Ok(FragmentType::AttestationOnEachEnd),
            _ => Err(format!("Invalid fragment type: {s}")),
        }
    }
}

#[derive(Debug, Clone)]
struct IntervalEndpoint {
    block_number: u64,
    digest: H256,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Cannot prove a query for source chain that hasn't been attested to.")]
    NoAttestationsSynced,
    #[error("Cannot prove queries more recent than latest attestation: Last attestation: {0}, query height: {1}")]
    QueryTooRecent(u64, u64),
    #[error("Could not get last block of fragment.")]
    NoLastBlockFound,
    #[error("End attestation digest does not match last fragment block digest, End attestation: {0}, Last fragment block: {1}, Fetched block from source chain: {2}")]
    LastFragmentBlockMismatch(H256, String, bool),
    #[error("Error appending block to fragment: {0:?}")]
    ErrorAppendingBlock(#[from] AttestationFragmentError),
    #[error("Could not get the highest checkpoint before query. Query height: {0}")]
    FailedToGetHighestCheckpointBefore(u64),
    #[error("Could not get the lowest checkpoint after query. Query height: {0}")]
    FailedToGetLowestCheckpointAfter(u64),
    #[error("Could not get the highest attestation before query. Query height: {0}")]
    FailedToGetHighestAttestationBefore(u64),
    #[error("Could not get the lowest attestation after query. Query height: {0}")]
    FailedToGetLowestAttestationAfter(u64),
    #[error("Prover DB error: {0}")]
    ProverDBError(String),
    #[error("{0}")]
    Other(String),
    #[error("Failed to parse fragment digest")]
    InvalidFragmentDigest,
    #[error("Failed to get chain key")]
    FailedToGetChainKey,
    #[error("Wrong chain: expected {0}, got {1}")]
    WrongChain(u64, u64),
    #[error("Fragment manager error: {0}")]
    FragmentManagerError(#[from] FragmentManagerError),
}

// Get the attestation fragment for a query
// This function will either get the fragment from the cache or create it and store it in the cache
// The fragment is created by querying the chain for the attestation chain interval and then querying the chain for the attestation fragment

pub async fn get_for_query(
    eth_client: &Client,
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<AttestationFragment, Error> {
    let chain_key = query.chain_id;

    // 1) Guard: the query must not be more recent than the latest attestation we synced
    let last_attestation_height = from_storage_type(
        attestation_cache
            .last_synced_attestation(chain_key)
            .await
            .map_err(|e| Error::ProverDBError(e.to_string()))?
            .ok_or(Error::NoAttestationsSynced)?
            .header_number,
    );
    if last_attestation_height < query.height {
        return Err(Error::QueryTooRecent(last_attestation_height, query.height));
    }

    // 2) Resolve interval [lower, upper] (exclusive of lower, inclusive of upper for block list)
    let (lower, upper) = get_endpoints_for_query(query, attestation_cache)
        .await
        .map_err(|e| Error::ProverDBError(e.to_string()))?;

    let expected_len = upper.block_number - lower.block_number;

    // 3) Try cache first
    let cached = attestation_cache
        .get_attestation_fragment(chain_key, lower.block_number, upper.block_number)
        .await
        .map_err(|e| Error::ProverDBError(e.to_string()))?;

    let fetched_end_from_chain = cached.last().map_or(true, |b| {
        from_storage_type(b.header_number) != upper.block_number
    });

    let blocks: Vec<BlockWithDigest> = if cached.len() as u64 == expected_len {
        debug!(
            "✅ Fragment found in cache (chain={}, {}..={})",
            chain_key, lower.block_number, upper.block_number
        );
        validate_end_digest(&cached, upper.digest, fetched_end_from_chain)?;
        cached
    } else {
        debug!(
            "🧱 Cache miss/partial (have {}, need {}): building via FragmentManager",
            cached.len(),
            expected_len
        );
        // Build once via FragmentManager
        let fragment = build_with_manager(eth_client, &lower, &upper).await?;
        validate_end_digest_fragment(&fragment, upper.digest, true)?;

        // Persist to cache (single upsert) and return the mapped blocks
        let mapped = map_fragment_blocks(chain_key, &fragment);
        attestation_cache
            .upsert_fragment(&mapped)
            .await
            .map_err(|e| Error::ProverDBError(e.to_string()))?;
        mapped
    };

    // 4) Turn blocks into AttestationFragment (functional, no explicit for-loop)
    blocks_to_fragment(&blocks, &lower)
}

/// Build fragment using `FragmentManager` in a single call.
async fn build_with_manager(
    eth_client: &Client,
    lower: &IntervalEndpoint,
    upper: &IntervalEndpoint,
) -> Result<AttestationFragment, Error> {
    let manager = FragmentManager::new(lower.block_number + 1, upper.block_number, eth_client);
    debug!("📝 Providing lower endpoint: {:?}", lower);
    let fragment = manager.create(lower.digest).await?;
    Ok(fragment)
}

/// Verify that the last block in `blocks` matches `expected_end`.
fn validate_end_digest(
    blocks: &[BlockWithDigest],
    expected_end: H256,
    fetched_end_from_chain: bool,
) -> Result<(), Error> {
    let last = blocks.last().ok_or(Error::NoLastBlockFound)?;
    let last_digest = H256::from_str(&last.digest).map_err(|_| Error::InvalidFragmentDigest)?;
    if last_digest != expected_end {
        return Err(Error::LastFragmentBlockMismatch(
            expected_end,
            last.digest.clone(),
            fetched_end_from_chain,
        ));
    }
    Ok(())
}

/// Same as above, but for a fresh `AttestationFragment`.
fn validate_end_digest_fragment(
    fragment: &AttestationFragment,
    expected_end: H256,
    fetched_end_from_chain: bool,
) -> Result<(), Error> {
    let last = fragment.blocks().last().ok_or(Error::NoLastBlockFound)?;
    let last_digest = H256::from(last.digest().to_bytes_be());
    if last_digest != expected_end {
        return Err(Error::LastFragmentBlockMismatch(
            expected_end,
            last_digest.encode_hex(),
            fetched_end_from_chain,
        ));
    }
    Ok(())
}

/// Map `AttestationFragment` blocks to your DB model for persistence.
fn map_fragment_blocks(chain_key: u64, fragment: &AttestationFragment) -> Vec<BlockWithDigest> {
    fragment
        .blocks()
        .iter()
        .map(|b| {
            let hash = H256::from(b.digest().to_bytes_be());
            BlockWithDigest {
                chain_key: chain_key as i64,
                digest: hash.encode_hex(),
                header_number: b.block_number as i64,
                merkle_root: hex::encode(b.root.to_bytes_be()),
            }
        })
        .collect()
}

/// Turn cached DB rows into `AttestationFragment` without a manual mutable loop.
/// Needs a lower endpoint to start with, which is used to compute the first block's previous digest.
fn blocks_to_fragment(
    blocks: &[BlockWithDigest],
    lower: &IntervalEndpoint,
) -> Result<AttestationFragment, Error> {
    let mut frag = AttestationFragment::new(blocks.len() + 1);

    // Append the first block with the lower endpoint's digest.
    // This is the first block in the fragment, so it has no previous digest.
    frag.try_append_block(Block::new_from_digest(
        lower.block_number,
        Felt::from_bytes_be(&lower.digest.0),
    ))?;

    // Keep the previous digest as an owned String we can update each step.
    let mut prev = lower.digest.encode_hex::<String>();

    for b in blocks {
        let block_with_prev =
            create_block_with_prev_digest(b, &prev).map_err(|e| Error::Other(e.to_string()))?;

        // compute next "prev" as hex string
        prev = hex::encode(block_with_prev.digest.to_bytes_be());

        // append to the fragment we're building
        frag.try_append_block(block_with_prev)?;
    }

    Ok(frag)
}

async fn get_endpoints_for_query(
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<(IntervalEndpoint, IntervalEndpoint)> {
    // Interval depends on whether the fragment in question ends with a checkpoint or an attestation.
    // Attestations occur strictly after checkpoints, since checkpoints remove all preceding
    // attestations. Thus we change how we calculate our interval based on the height of the query
    // block.
    let fragment_type = fragment_type(query, attestation_cache).await?;
    debug!(
        "🔍 Interval bounds found for fragment type: {:?}",
        fragment_type
    );
    fetch_interval_ends(query, fragment_type, attestation_cache).await
}

pub async fn get_fragment_type_for_query(
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<FragmentType> {
    fragment_type(query, attestation_cache).await
}

async fn fragment_type(
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<FragmentType> {
    let maybe_latest_checkpoint = attestation_cache
        .currently_cached_up_to(query.chain_id)
        .await?;

    if let Some(latest_checkpoint) = maybe_latest_checkpoint {
        debug!("📈 Latest checkpoint: {:?}", latest_checkpoint.digest);
        let last_checkpoint = attestation_cache
            .get_checkpoint_by_digest(latest_checkpoint.digest)
            .await?;
        let latest_checkpoint_height = postgres::from_storage_type(last_checkpoint.block_number);
        match latest_checkpoint_height {
            last_check if last_check >= query.height => {
                // Query is in checkpoint fragment
                Ok(FragmentType::CheckpointOnEachEnd)
            }
            _ => Ok(FragmentType::AttestationOnEachEnd),
        }
    } else {
        // If there are no checkpoints, then fragment must start and end with attestation
        Ok(FragmentType::AttestationOnEachEnd)
    }
}

async fn fetch_interval_ends(
    query: &Query,
    fragment_type: FragmentType,
    attestation_cache: &AttestationCacheType,
) -> Result<(IntervalEndpoint, IntervalEndpoint)> {
    // Get digests corresponding to starting and ending checkpoints/attestations
    match fragment_type {
        FragmentType::CheckpointOnEachEnd => {
            let checkpoint = attestation_cache
                .get_highest_checkpoint_before(query.height, query.chain_id)
                .await?
                .ok_or(Error::FailedToGetHighestCheckpointBefore(query.height))?;
            let start = IntervalEndpoint {
                block_number: from_storage_type(checkpoint.block_number),
                digest: H256::from_str(&checkpoint.digest)?,
            };
            let checkpoint = attestation_cache
                .get_lowest_checkpoint_after(query.height, query.chain_id)
                .await?
                .ok_or(Error::FailedToGetLowestCheckpointAfter(query.height))?;
            let end = IntervalEndpoint {
                block_number: from_storage_type(checkpoint.block_number),
                digest: H256::from_str(&checkpoint.digest)?,
            };

            Ok((start, end))
        }
        FragmentType::AttestationOnEachEnd => {
            let start: IntervalEndpoint = if let Some(start_attestation) = attestation_cache
                .get_highest_attestation_before(query.height, query.chain_id)
                .await?
            {
                IntervalEndpoint {
                    block_number: from_storage_type(start_attestation.header_number),
                    digest: H256::from_str(&start_attestation.digest)?,
                }
            } else {
                // Corner case can result in first attestation being removed before its corresponding checkpoint is
                // created. In this case we use the first checkpoint instead.
                let start_checkpoint = attestation_cache
                    .get_highest_checkpoint_before(query.height, query.chain_id)
                    .await?
                    .ok_or(Error::FailedToGetHighestAttestationBefore(query.height))?;
                IntervalEndpoint {
                    block_number: from_storage_type(start_checkpoint.block_number),
                    digest: H256::from_str(&start_checkpoint.digest)?,
                }
            };

            let end_attestation = attestation_cache
                .get_lowest_attestation_after(query.height, query.chain_id)
                .await?
                .ok_or(Error::FailedToGetLowestAttestationAfter(query.height))?;
            let end = IntervalEndpoint {
                block_number: from_storage_type(end_attestation.header_number),
                digest: H256::from_str(&end_attestation.digest)?,
            };

            Ok((start, end))
        }
    }
}
