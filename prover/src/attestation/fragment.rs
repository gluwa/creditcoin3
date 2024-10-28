use anyhow::Result;
use hex::ToHex;
use sp_core::H256;
use thiserror::Error;
use tracing::{debug, info};

use attestation_chain::attestation_fragment::{AttestationFragment, AttestationFragmentError};
use attestor_primitives::Attestation as AttestationPrimitive;
use eth::Client;
use mmr::traits::MerkleTreeTrait;
use pallet_prover_primitives::Query;

use crate::attestation::create_block_with_prev_digest;
use crate::postgres::blockwithdigest::BlockWithDigest;
use crate::postgres::from_storage_type;
use crate::{postgres, AttestationCacheType, EthClientArc};

#[derive(Debug)]
enum FragmentType {
    /// All fragments ending in a checkpoint also start with a checkpoint
    CheckpointOnEachEnd,
    /// Most fragments ending in an attestation also start with an attestation
    AttestationOnEachEnd,
}

#[derive(Debug, Clone)]
struct IntervalEndpoint {
    block_number: u64,
    digest: String,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Cannot prove a claim for source chain that hasn't been attested to.")]
    NoAttestationsSynced,
    #[error("Cannot prove queries more recent than latest attestation: Last attestation: {0}, claim height: {1}")]
    QueryTooRecent(u64, u64),
    #[error("Could not get first block of fragment.")]
    NoFirstBlockFound,
    #[error("Start attestation digest does not match first fragment block digest, Start attestation: {0}, First fragment block: {1}")]
    FirstFragmentBlockMismatch(String, String),
    #[error("Could not get last block of fragment.")]
    NoLastBlockFound,
    #[error("End attestation digest does not match last fragment block digest, End attestation: {0}, Last fragment block: {1}")]
    LastFragmentBlockMismatch(String, String),
    #[error("Error appending block to fragment: {0:?}")]
    ErrorAppendingBlock(#[from] AttestationFragmentError),
    #[error("Could not get the highest checkpoint before claim. Claim height: {0}")]
    FailedToGetHighestCheckpointBefore(u64),
    #[error("Could not get the lowest checkpoint after claim. Claim height: {0}")]
    FailedToGetLowestCheckpointAfter(u64),
    #[error("Could not get the highest attestation before claim. Claim height: {0}")]
    FailedToGetHighestAttestationBefore(u64),
    #[error("Could not get the lowest attestation after claim. Claim height: {0}")]
    FailedToGetLowestAttestationAfter(u64),
    #[error("Prover DB error: {0}")]
    ProverDBError(String),
    #[error("{0}")]
    Other(String),
}

// Get the attestation fragment for a claim
// This function will either get the fragment from the cache or create it and store it in the cache
// The fragment is created by querying the chain for the attestation chain interval and then querying the chain for the attestation fragment
pub async fn get_for_claim(
    eth_client: &EthClientArc,
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> std::result::Result<AttestationFragment, Error> {
    let chain_key = query.chain_id;

    // Before processing claim, check that it is for a valid block number. All blocks up
    // to the height of the latest attestation should be valid.
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

    // Fetch interval ends for the attestation chain
    let (lower_endpoint, upper_endpoint) = get_endpoints_for_claim(query, attestation_cache)
        .await
        .map_err(|e| Error::ProverDBError(e.to_string()))?;

    let expected_fragment_size = upper_endpoint.block_number - lower_endpoint.block_number + 1;

    // Get fragment from cache
    let db_fragment = attestation_cache
        .get_attestation_fragment(
            chain_key,
            lower_endpoint.block_number,
            upper_endpoint.block_number,
        )
        .await
        .map_err(|e| Error::ProverDBError(e.to_string()))?;

    // If not all fragment blocks are in the cache, then add them.
    let fragment_blocks = if db_fragment.len() as u64 == expected_fragment_size {
        db_fragment
    } else {
        construct_fragment(
            db_fragment,
            eth_client,
            chain_key,
            lower_endpoint.block_number,
            upper_endpoint.block_number,
        )
        .await
        .map_err(|e| Error::Other(e.to_string()))?
    };

    // Sanity check that the start attestation digest matches the first block in the fragment
    let first_fragment_block = fragment_blocks
        .first()
        .ok_or_else(|| Error::NoFirstBlockFound)?;

    if lower_endpoint.digest.as_bytes() != first_fragment_block.digest.as_bytes() {
        return Err(Error::FirstFragmentBlockMismatch(
            lower_endpoint.digest,
            first_fragment_block.digest.clone(),
        ));
    };

    // Sanity check that the end attestation digest matches the last fragment block digest
    let last_fragment_block = fragment_blocks
        .last()
        .ok_or_else(|| Error::NoLastBlockFound)?;

    if upper_endpoint.digest.as_bytes() != last_fragment_block.digest.as_bytes() {
        return Err(Error::LastFragmentBlockMismatch(
            upper_endpoint.digest,
            last_fragment_block.digest.clone(),
        ));
    };

    // Store the fragment in the cache
    attestation_cache
        .upsert_fragment(&fragment_blocks)
        .await
        .map_err(|e| Error::ProverDBError(e.to_string()))?;

    // Initialize the attestation fragment
    let mut attestation_fragment = AttestationFragment::new(
        usize::try_from(expected_fragment_size).map_err(|e| Error::Other(e.to_string()))?,
    );

    // We use a dummy value rather than an Option::None here for simplicity
    let mut prev_block_digest: String = H256::zero().encode_hex();

    // Construct the attestation fragment
    for fragment_block in &fragment_blocks {
        let block_with_prev_digest =
            create_block_with_prev_digest(fragment_block, &prev_block_digest)
                .map_err(|e| Error::Other(e.to_string()))?;

        // Update prev digest for the next block
        prev_block_digest = hex::encode(block_with_prev_digest.digest.to_bytes_be());

        attestation_fragment.try_append_block(block_with_prev_digest)?;
    }

    Ok(attestation_fragment)
}

// Construct a list of blocks for a given chain_key and interval. This function will query
// the source chain for the blocks in the interval and then generate digests for those
// blocks. They are mapped to the database model and returned.
async fn construct_fragment(
    already_in_cache: Vec<BlockWithDigest>,
    eth_client: &Client,
    chain_key: u64,
    lower_bound: u64,
    upper_bound: u64,
) -> Result<Vec<BlockWithDigest>> {
    info!(
        "Not all blocks of attestation fragment found in cache, creating fragment for chain_key: {}, lower_bound: {}, upper_bound: {}",
        chain_key, lower_bound, upper_bound
    );

    let mut fragment_blocks = vec![];
    // Get every block from upper_bound to lower_bound from eth client
    // TODO: This should be done in parallel
    let mut prev_digest = None;
    for block_number in lower_bound..=upper_bound {
        if let Some(block) = already_in_cache
            .iter()
            .find(|block| postgres::from_storage_type(block.header_number) == block_number)
        {
            fragment_blocks.push(block.clone());
            continue;
        }
        let block = eth_client.get_block(block_number).await?;

        // Generate the pedersen mmr
        let mt = eth::starknet_pedersen_mmr(&block);

        // Create the primitive to generate a digest after
        let attestation_primitive = AttestationPrimitive {
            chain_key,
            header_hash: block.hash().unwrap(),
            header_number: block_number,
            prev_digest,
            root: mt.root().0.to_bytes_be(),
        };

        // Get the digest of the source chain block
        let digest = attestation_primitive.digest();
        debug!(
            "Block with digest for chain_key: {}, header_number: {}, digest: {}",
            chain_key, attestation_primitive.header_number, digest
        );

        // Update the prev_digest
        prev_digest = Some(digest);

        // Convert each block to type including digest
        let block_with_digest = BlockWithDigest {
            chain_key: chain_key as i64,
            header_number: attestation_primitive.header_number as i64,
            digest: hex::encode(digest.as_bytes()),
            header_hash: attestation_primitive
                .header_hash
                .to_string()
                .strip_prefix("0x")
                .unwrap()
                .to_string(),
            merkle_root: hex::encode(mt.root().0.to_bytes_be()),
        };

        fragment_blocks.push(block_with_digest);
    }

    Ok(fragment_blocks)
}

async fn get_endpoints_for_claim(
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<(IntervalEndpoint, IntervalEndpoint)> {
    // Interval depends on whether the fragment in question ends with a checkpoint or an attestation.
    // Attestations occur strictly after checkpoints, since checkpoints remove all preceding
    // attestations. Thus we change how we calculate our interval based on the height of the query
    // block.
    let fragment_type = fragment_type(query, attestation_cache).await?;
    info!(
        "Interval bounds found for fragment type: {:?}",
        fragment_type
    );
    fetch_interval_ends(query, fragment_type, attestation_cache).await
}

async fn fragment_type(
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<FragmentType> {
    let maybe_latest_checkpoint = attestation_cache
        .currently_cached_up_to(query.chain_id)
        .await?;

    if let Some(latest_checkpoint) = maybe_latest_checkpoint {
        info!("Latest checkpoint: {:?}", latest_checkpoint.digest);
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
                digest: checkpoint.digest,
            };
            let checkpoint = attestation_cache
                .get_lowest_checkpoint_after(query.height, query.chain_id)
                .await?
                .ok_or(Error::FailedToGetLowestCheckpointAfter(query.height))?;
            let end = IntervalEndpoint {
                block_number: from_storage_type(checkpoint.block_number),
                digest: checkpoint.digest,
            };

            Ok((start, end))
        }
        FragmentType::AttestationOnEachEnd => {
            let start_attestation = attestation_cache
                .get_highest_attestation_before(query.height, query.chain_id)
                .await?
                .ok_or(Error::FailedToGetHighestAttestationBefore(query.height))?;
            let start = IntervalEndpoint {
                block_number: from_storage_type(start_attestation.header_number),
                digest: start_attestation.digest,
            };

            let end_attestation = attestation_cache
                .get_lowest_attestation_after(query.height, query.chain_id)
                .await?
                .ok_or(Error::FailedToGetLowestAttestationAfter(query.height))?;
            let end = IntervalEndpoint {
                block_number: from_storage_type(end_attestation.header_number),
                digest: end_attestation.digest,
            };

            Ok((start, end))
        }
    }
}
