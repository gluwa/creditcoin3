use anyhow::anyhow;
use anyhow::Result;
use hex::ToHex;
use sp_core::H256;
use tracing::{debug, info};

use attestation_chain::attestation_fragment::AttestationFragment;
use attestor_primitives::Attestation as AttestationPrimitive;
use eth::Client;
use mmr::traits::MerkleTreeTrait;
use pallet_prover_primitives::Query;

use crate::attestation::create_block_with_prev_digest;
use crate::postgres::blockwithdigest::BlockWithDigest;
use crate::postgres::from_storage_type;
use crate::{postgres, AttestationCacheType, EthClientArc};

#[derive(Debug)]
pub enum FragmentType {
    /// All fragments ending in a checkpoint also start with a checkpoint
    CheckpointOnEachEnd,
    /// Most fragments ending in an attestation also start with an attestation
    AttestationOnEachEnd,
    /// The fragment after the most recent checkpoint will start with an attestation
    /// but end with a checkpoint.
    StartCheckpointEndAttestation,
}

// Get the attestation fragment for a claim
// This function will either get the fragment from the cache or create it and store it in the cache
// The fragment is created by querying the chain for the attestation chain interval and then querying the chain for the attestation fragment
pub async fn get_for_claim(
    eth_client: &EthClientArc,
    query: &Query,
    attestation_cache: &AttestationCacheType,
    attestation_interval: u64,
    checkpoint_interval: u64,
) -> Result<AttestationFragment> {
    let chain_id = query.chain_id;

    // Calculate the interval bounds for the attestation chain
    let (lower_bound, upper_bound, fragment_type) = get_interval_bounds(
        query.height,
        attestation_interval,
        checkpoint_interval,
        attestation_cache,
    )
    .await?;

    let expected_fragment_size = upper_bound - lower_bound + 1;

    let last_attestation_height = from_storage_type(
        attestation_cache
            .last_synced_attestation(chain_id)
            .await?
            .ok_or(anyhow!("No attestations synced!"))?
            .header_number,
    );

    if last_attestation_height < query.height {
        return Err(anyhow!(
            "Cannot prove queries more recent than latest attestation"
        ));
    }

    // Get digests corresponding to starting and ending checkpoints/attestations
    let start_digest = match fragment_type {
        FragmentType::CheckpointOnEachEnd | FragmentType::StartCheckpointEndAttestation => {
            attestation_cache
                .get_checkpoint_by_block_number(lower_bound as i64, chain_id as i64)
                .await?
                .digest
        }
        FragmentType::AttestationOnEachEnd => {
            attestation_cache
                .get_attestation_by_header_number(lower_bound as i64, chain_id as i64)
                .await?
                .digest
        }
    };
    let end_digest = match fragment_type {
        FragmentType::CheckpointOnEachEnd => {
            attestation_cache
                .get_checkpoint_by_block_number(upper_bound as i64, chain_id as i64)
                .await?
                .digest
        }
        FragmentType::AttestationOnEachEnd | FragmentType::StartCheckpointEndAttestation => {
            attestation_cache
                .get_attestation_by_header_number(upper_bound as i64, chain_id as i64)
                .await?
                .digest
        }
    };

    // Get fragment from cache
    let db_fragment = attestation_cache
        .get_attestation_fragment(chain_id, lower_bound, upper_bound)
        .await?;

    // If not all fragment blocks are in the cache, then add them.
    let fragment_blocks = if db_fragment.len() as u64 == expected_fragment_size {
        db_fragment
    } else {
        construct_fragment(db_fragment, eth_client, chain_id, lower_bound, upper_bound).await?
    };

    // Sanity check that the start attestation digest matches the first block in the fragment
    let first_fragment_block = fragment_blocks
        .first()
        .ok_or_else(|| anyhow!("No first block found"))?;

    if start_digest.as_bytes() != first_fragment_block.digest.as_bytes() {
        return Err(anyhow!(
            "Start attestation digest does not match first fragment block digest: Start attestation: {:?}, First fragment block: {:?}",
            start_digest,
            first_fragment_block.digest
        ));
    };

    // Sanity check that the end attestation digest matches the last fragment block digest
    let last_fragment_block = fragment_blocks
        .last()
        .ok_or_else(|| anyhow!("No last block found"))?;

    if end_digest.as_bytes() != last_fragment_block.digest.as_bytes() {
        return Err(anyhow!(
            "End attestation digest does not match last fragment block digest:\n
            End attestation: {:?}
            \nLast fragment block: {:?}\n",
            end_digest,
            last_fragment_block.digest
        ));
    };

    // Store the fragment in the cache
    attestation_cache.upsert_fragment(&fragment_blocks).await?;

    // Initialize the attestation fragment
    let mut attestation_fragment =
        AttestationFragment::new(usize::try_from(expected_fragment_size)?);

    // We use a dummy value rather than an Option::None here for simplicity
    let mut prev_block_digest: String = H256::zero().encode_hex();

    // Construct the attestation fragment
    for fragment_block in &fragment_blocks {
        let block_with_prev_digest =
            create_block_with_prev_digest(fragment_block, &prev_block_digest)?;

        // Update prev digest for the next block
        prev_block_digest = hex::encode(block_with_prev_digest.digest.to_bytes_be());

        attestation_fragment
            .try_append_block(block_with_prev_digest)
            .map_err(|e| anyhow!("Error appending block to fragment: {:?}", e))?;
    }

    Ok(attestation_fragment)
}

// Construct a list of blocks for a given chain_id and interval. This function will query
// the source chain for the blocks in the interval and then generate digests for those
// blocks. They are mapped to the database model and returned.
async fn construct_fragment(
    already_in_cache: Vec<BlockWithDigest>,
    eth_client: &Client,
    chain_id: u64,
    lower_bound: u64,
    upper_bound: u64,
) -> Result<Vec<BlockWithDigest>> {
    info!(
        "Not all blocks of attestation fragment found in cache, creating fragment for chain_id: {}, lower_bound: {}, upper_bound: {}",
        chain_id, lower_bound, upper_bound
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
            chain_id,
            header_hash: block.hash().unwrap(),
            header_number: block_number,
            prev_digest,
            root: mt.root().0.to_bytes_be(),
        };

        // Get the digest of the source chain block
        let digest = attestation_primitive.digest();
        debug!(
            "Block with digest for chain_id: {}, header_number: {}, digest: {}",
            chain_id, attestation_primitive.header_number, digest
        );

        // Update the prev_digest
        prev_digest = Some(digest);

        // Convert each block to type including digest
        let block_with_digest = BlockWithDigest {
            chain_id: chain_id as i64,
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

async fn get_interval_bounds(
    query_height: u64,
    attestation_interval: u64,
    checkpoint_interval: u64,
    attestation_cache: &AttestationCacheType,
) -> Result<(u64, u64, FragmentType)> {
    let maybe_latest_checkpoint = attestation_cache.currently_cached_up_to().await?;
    // Interval depends on whether the fragment in question ends with a checkpoint or an attestation.
    // Attestations occur strictly after checkpoints, since checkpoints remove all preceding
    // attestations. Thus we change how we calculate our interval based on the height of the query
    // block.
    let (total_interval, fragment_type) = if let Some(latest_checkpoint) = maybe_latest_checkpoint {
        info!("Latest checkpoint: {:?}", latest_checkpoint.digest);
        let last_checkpoint = attestation_cache
            .get_checkpoint_by_digest(latest_checkpoint.digest)
            .await?;
        let latest_checkpoint_height = postgres::from_storage_type(last_checkpoint.block_number);
        match latest_checkpoint_height {
            h if h >= query_height => {
                // Query is in checkpoint fragment
                (
                    attestation_interval * checkpoint_interval,
                    FragmentType::CheckpointOnEachEnd,
                )
            }
            h if h + attestation_interval >= query_height => {
                // Query is in first attestation interval after checkpoint
                (
                    attestation_interval,
                    FragmentType::StartCheckpointEndAttestation,
                )
            }
            _ => (attestation_interval, FragmentType::AttestationOnEachEnd),
        }
    } else {
        // If there are no checkpoints, then fragment must start and end with attestation
        (attestation_interval, FragmentType::AttestationOnEachEnd)
    };

    info!(
        "Interval bounds found for fragment type: {:?}",
        fragment_type
    );

    // We can now actually calculate the interval bounds. First we round
    // down to the start of the attestation or checkpoint interval.
    let start = (query_height / total_interval) * total_interval;
    let end = start + total_interval;
    Ok((start, end, fragment_type))
}
