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
use crate::{postgres, AttestationCacheType, EthClientArc};

// Get the attestation fragment for a claim
// This function will either get the fragment from the cache or create it and store it in the cache
// The fragment is created by querying the chain for the attestation chain interval and then querying the chain for the attestation fragment
pub async fn get_for_claim(
    eth_client: &EthClientArc,
    query: &Query,
    attestation_cache: &AttestationCacheType,
    attestation_interval: u64,
    checkpoint_interval: u32,
) -> Result<AttestationFragment> {
    let chain_id = query.chain_id;

    // Calculate the interval bounds for the attestation chain
    let (lower_bound, upper_bound) = get_interval_bounds(
        query.height,
        attestation_interval,
        checkpoint_interval,
        attestation_cache,
    )
    .await?;

    // Get the checkpoints for the interval
    let start_attestation = attestation_cache
        .get_attestation_by_header_number(lower_bound as i64, chain_id as i64)
        .await?;
    let end_attestation = attestation_cache
        .get_attestation_by_header_number(upper_bound as i64, chain_id as i64)
        .await?;

    // Get fragment from cache
    let db_fragment = attestation_cache
        .get_attestation_fragment(chain_id, lower_bound, upper_bound)
        .await?;

    // If not all fragment blocks are in the cache, then add them.
    let fragment_blocks = if db_fragment.len() != (upper_bound - lower_bound) as usize {
        construct_fragment(db_fragment, eth_client, chain_id, lower_bound, upper_bound).await?
    } else {
        db_fragment
    };

    info!(
        "Attestation fragment found in cache, fragment length: {}",
        fragment_blocks.len()
    );

    // Check if the first block digest matches the start attestation digest
    let first_fragment_block = fragment_blocks
        .first()
        .ok_or_else(|| anyhow::anyhow!("No first block found"))?;
    info!(
        "First fragment block digest: {}",
        first_fragment_block.digest
    );

    if start_attestation.digest.as_bytes() != first_fragment_block.digest.as_bytes() {
        debug!(
            "Start attestation digest: {}, first attestation digest: {}",
            start_attestation.digest, first_fragment_block.digest
        );
        return Err(anyhow::anyhow!(
            "Start attestation digest does not match first fragment block digest"
        ));
    };

    // Check if the last block digest matches the end attestation digest
    let last_fragment_block = fragment_blocks
        .last()
        .ok_or_else(|| anyhow::anyhow!("No last block found"))?;
    info!("Last fragment block digest: {}", last_fragment_block.digest);

    if end_attestation.digest.as_bytes() != last_fragment_block.digest.as_bytes() {
        debug!(
            "End attestation digest: {}, last fragment block digest: {}",
            end_attestation.digest, last_fragment_block.digest
        );
        return Err(anyhow::anyhow!(
            "End attestation digest does not match last fragment block digest"
        ));
    };

    // Everything checks out, upsert the fragment in the Database so the next time we don't have to create it
    attestation_cache.upsert_fragment(&fragment_blocks).await?;

    // Create the attestation fragment object
    let mut attestation_fragment = AttestationFragment::new(
        usize::try_from(upper_bound - lower_bound).expect("Interval is too large"),
    );

    // First digest is the start checkpoint
    let mut start_digest = start_attestation
        .prev_digest
        // If the start checkpoint has no prev digest, use the zero digest
        // Only in the case of the first checkpoint
        .unwrap_or(H256::zero().encode_hex());

    info!(
        "Start digest: {}, End digest: {}",
        start_digest, end_attestation.digest
    );
    for fragment_block in &fragment_blocks {
        let block_with_prev_digest = create_block_with_prev_digest(fragment_block, &start_digest)?;
        info!(
            "Appending block to fragment. chain_id: {}, block: {:?}",
            chain_id, block_with_prev_digest
        );

        // Update the digest
        start_digest = hex::encode(block_with_prev_digest.digest.to_bytes_be());
        info!("Hex start digest (updated): {:?}", start_digest);

        attestation_fragment
            .try_append_block(block_with_prev_digest)
            .map_err(|e| anyhow::anyhow!("Error appending block to fragment: {:?}", e))?;
    }

    info!(
        "Attestation fragment created for chain_id: {}, lower_bound: {}, upper_bound: {}",
        chain_id, lower_bound, upper_bound
    );

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
    // Get every block for upper_bound to lower_bound from eth client
    // TODO: This should be done in parallel
    let mut prev_digest = None;
    for block_number in lower_bound..=upper_bound {
        if let Some(block) = already_in_cache
            .iter()
            .filter(|block| postgres::from_storage_type(block.header_number) == block_number)
            .next()
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
    checkpoint_interval: u32,
    attestation_cache: &AttestationCacheType,
) -> Result<(u64, u64)> {
    let mut latest_checkpoint_height = 0;
    let maybe_latest_checkpoint = attestation_cache.currently_cached_up_to().await?;
    // Interval depends on whether the fragment in question ends with a checkpoint or an attestation.
    // Attestations occur strictly after checkpoints, since checkpoints remove all preceding
    // attestations. Thus we change how we calculate our interval based on the height of the query
    // block.
    let (total_interval, is_checkpoint_fragment) =
        if let Some(latest_checkpoint) = maybe_latest_checkpoint {
            let hex_digest = H256::from_slice(latest_checkpoint.digest.as_bytes());
            let last_checkpoint = attestation_cache
                .get_checkpoint_by_digest(hex_digest)
                .await?;
            latest_checkpoint_height = postgres::from_storage_type(last_checkpoint.block_number);
            if latest_checkpoint_height >= query_height {
                // Query is in checkpoint fragment
                (attestation_interval * checkpoint_interval as u64, true)
            } else {
                (attestation_interval, false)
            }
        } else {
            (attestation_interval, false)
        };

    // We can now actually calculate the interval bounds
    if is_checkpoint_fragment {
        let start = (query_height / total_interval) * total_interval;
        let end = start + total_interval;
        Ok((start, end))
    } else {
        let start = ((query_height - latest_checkpoint_height) / attestation_interval)
            * attestation_interval;
        let end = start + attestation_interval;
        Ok((start, end))
    }
}
