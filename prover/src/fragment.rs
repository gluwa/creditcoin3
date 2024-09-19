use anyhow::Result;
use hex::ToHex;
use sp_core::H256;
use tracing::{debug, info};

use attestation_chain::{attestation_fragment::AttestationFragment, AttestationChainParams};
use attestor_primitives::Attestation as AttestationPrimitive;
use eth::Client;
use mmr::traits::MerkleTreeTrait;
use prover_primitives::Query;

use crate::attestation::create_block_attestation;
use crate::{postgres, AttestationCacheType, EthClientArc};

// Get the attestation fragment for a claim
// This function will either get the fragment from the cache or create it and store it in the cache
// The fragment is created by querying the chain for the attestation chain interval and then querying the chain for the attestation fragment
pub async fn get_for_claim(
    eth_client: &EthClientArc,
    query: &Query,
    attestation_cache: &AttestationCacheType,
    interval: u64,
) -> Result<AttestationFragment> {
    let chain_id = query.chain_id;

    // Calculate the interval bounds for the attestation chain
    let (lower_bound, upper_bound) = get_interval_bounds(query.height, interval);

    // Get the checkpoints for the interval
    let start_checkpoint = attestation_cache
        .get_by_header_number(lower_bound as i64, chain_id as i64)
        .await?;
    let end_checkpoint = attestation_cache
        .get_by_header_number(upper_bound as i64, chain_id as i64)
        .await?;

    // Get fragment from cache
    let db_fragment = attestation_cache
        .get_attestation_fragment(chain_id, lower_bound, upper_bound)
        .await?;

    // If the fragment is not in the cache, create it
    let attestations = if db_fragment.is_empty() {
        construct_fragment(eth_client, chain_id, lower_bound, upper_bound).await?
    } else {
        db_fragment
    };

    info!(
        "Attestation fragment found in cache, fragment length: {}",
        attestations.len()
    );

    // Check if the first attestation digest matches the start checkpoint digest
    let first_attestation = attestations
        .first()
        .ok_or_else(|| anyhow::anyhow!("No first attestation found"))?;
    info!("First attestation digest: {}", first_attestation.digest);

    if start_checkpoint.digest.as_bytes() != first_attestation.digest.as_bytes() {
        debug!(
            "Start checkpoint digest: {}, first attestation digest: {}",
            start_checkpoint.digest, first_attestation.digest
        );
        return Err(anyhow::anyhow!(
            "Start checkpoint digest does not match first attestation digest"
        ));
    };

    // Check if the last attestation digest matches the end checkpoint digest
    let last_attestation = attestations
        .last()
        .ok_or_else(|| anyhow::anyhow!("No last attestation found"))?;
    info!("Last attestation digest: {}", last_attestation.digest);

    if end_checkpoint.digest.as_bytes() != last_attestation.digest.as_bytes() {
        debug!(
            "End checkpoint digest: {}, last attestation digest: {}",
            end_checkpoint.digest, last_attestation.digest
        );
        return Err(anyhow::anyhow!(
            "End checkpoint digest does not match last attestation digest"
        ));
    };

    // Everything checks out, upsert the fragment in the Database so the next time we don't have to create it
    attestation_cache.upsert_fragment(&attestations).await?;

    // Create the attestation fragment object
    let mut attestation_fragment = AttestationFragment::new(AttestationChainParams::new(
        0,
        usize::try_from(interval).expect("Interval is too large"),
    ));

    // First digest is the start checkpoint
    let mut start_digest = start_checkpoint
        .prev_digest
        // If the start checkpoint has no prev digest, use the zero digest
        // Only in the case of the first checkpoint
        .unwrap_or(H256::zero().encode_hex());

    info!(
        "Start digest: {}, End digest: {}",
        start_digest, end_checkpoint.digest
    );
    for attestation in &attestations {
        let block_attestation = create_block_attestation(attestation, &start_digest)?;
        info!(
            "Appending block attestation for chain_id: {}, block: {:?}",
            chain_id, block_attestation
        );

        // Update the digest
        start_digest = hex::encode(block_attestation.digest.to_bytes_be());
        info!("Hex start digest (updated): {:?}", start_digest);

        attestation_fragment
            .try_append_block(block_attestation)
            .map_err(|e| anyhow::anyhow!("Error appending block to fragment: {:?}", e))?;
    }

    info!(
        "Attestation fragment created for chain_id: {}, lower_bound: {}, upper_bound: {}",
        chain_id, lower_bound, upper_bound
    );

    Ok(attestation_fragment)
}

// Construct a list of attestations for a given chain_id and interval
// This function will query the source chain for the blocks in the interval and then generate the attestations
// They are mapped to the database model and returned
async fn construct_fragment(
    eth_client: &Client,
    chain_id: u64,
    lower_bound: u64,
    upper_bound: u64,
) -> Result<Vec<postgres::attestation::Attestation>> {
    info!(
        "Attestation fragment not found in cache, creating fragment for chain_id: {}, lower_bound: {}, upper_bound: {}",
        chain_id, lower_bound, upper_bound
    );

    let mut attestations = vec![];
    // Get every block for upper_bound to lower_bound from eth client
    // TODO: This should be done in parallel
    let mut prev_digest = None;
    for block_number in lower_bound..=upper_bound {
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

        // Get the digest of the attestation
        let digest = attestation_primitive.digest();
        debug!(
            "Attestation for chain_id: {}, header_number: {}, digest: {}",
            chain_id, attestation_primitive.header_number, digest
        );

        // Update the prev_digest
        prev_digest = Some(digest);

        // Create attestation for each block
        let attestation = postgres::attestation::Attestation {
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

        attestations.push(attestation);
    }

    Ok(attestations)
}

fn get_interval_bounds(number: u64, interval: u64) -> (u64, u64) {
    let start = (number / interval) * interval;
    let end = start + interval;
    (start, end)
}
