use anyhow::Result;
use either::Either;
use sp_core::H256;
use std::{ops::Range, path::PathBuf, time::Duration};
use thiserror::Error;
use tracing::{error, info, warn};

use pallet_prover_primitives::Query;
use prover_primitives::claim::{ClaimIdentifier, ClaimSerializable};

use crate::{attestation::fragment, AttestationCacheType, EthClientArc};

pub mod external;

/// Proof as bytes
pub type Proof = Vec<u8>;

/// Query id
pub type QueryId = H256;

const MAX_RETRIES: u32 = 10;

// todo: calculate this by averaging the last X block times on a source chain
const BLOCK_TIME: Duration = Duration::from_secs(6);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get proof")]
    Proof,
    #[error("Failed to get query files")]
    QueryFiles,
    #[error("Ethereum error: {0:?}")]
    EthError(#[from] eth::Error),
    #[error("Fragment error: {0:?}")]
    FragmentError(#[from] fragment::Error),
    #[error("Json error: {0:?}")]
    Json(#[from] serde_json::Error),
}

// Process a query
// Parameters:
// - `cc3_client`: cc3 client
// - `eth_client`: eth client that is initialized with the rpc url of the chain the claim is from
// - `query`: query to process
// - `attestation_cache`: attestation cache
// - `stone_proof`: whether to generate a stone proof
pub async fn process(
    eth_client: EthClientArc,
    query: &Query,
    attestation_cache: &AttestationCacheType,
    stone_proof: bool,
    chain_attestation_interval: u64,
) -> Result<Either<Proof, Vec<PathBuf>>, Error> {
    let query_id = query.id();
    info!("Processing query with id: {:?}", query_id);

    let mut retry_count = 0;

    // calculate how long we want to wait in the worst case scenario.
    // we want to wait for 2 chain attestation intervals
    let total_retry_delay =
        Duration::from_secs(chain_attestation_interval * BLOCK_TIME.as_secs() * 2);

    // Get the attestation fragment with retries on QueryTooRecent
    let attestation_fragment = loop {
        match fragment::get_for_claim(&eth_client, query, attestation_cache).await {
            Ok(fragment) => break fragment,
            Err(fragment::Error::QueryTooRecent(last_height, query_height))
                if retry_count < MAX_RETRIES =>
            {
                retry_count += 1;
                error!(
                    "QueryTooRecent error for query {:?}: last_attestation_height={}, query_height={}. Retry {}/{} in {:?}.",
                    query_id, last_height, query_height, retry_count, MAX_RETRIES, total_retry_delay/MAX_RETRIES
                );
                tokio::time::sleep(total_retry_delay / MAX_RETRIES).await;
            }
            Err(fragment::Error::FirstFragmentBlockMismatch(
                start_attestation,
                first_fragment_block,
                fetched_from_source,
            )) => {
                if fetched_from_source {
                    panic!("First fragment block fetched from source chain doesn't match attestation or checkpoint in prover DB. This means the source chain endpoint is untrustworthy or more likely the prover DB has invalid contents. Clean DB and run prover to resync. Start attestation: {start_attestation}, First fragment block: {first_fragment_block}")
                } else {
                    panic!("Digests from first fragment block and start attestation in DB don't match. The DB therefore contains invalid contents. Clean DB and run prover to resync. Start attestation: {start_attestation}, First fragment block: {first_fragment_block}")
                }
            }
            Err(fragment::Error::LastFragmentBlockMismatch(
                end_attestation,
                last_fragment_block,
                fetched_from_source,
            )) => {
                if fetched_from_source {
                    panic!("Last fragment block fetched from source chain doesn't match attestation or checkpoint in prover DB. This means the source chain endpoint is untrustworthy or more likely the prover DB has invalid contents. Clean DB and run prover to resync. End attestation: {end_attestation}, Last fragment block: {last_fragment_block}")
                } else {
                    panic!("Digests from last fragment block and end attestation in DB don't match. The DB therefore contains invalid contents. Clean DB and run prover to resync. End attestation: {end_attestation}, Last fragment block: {last_fragment_block}")
                }
            }
            Err(e) => return Err(e.into()),
        }
    };

    info!("Got attestation fragment for query with id: {:?}", query_id);

    let claim_serializable = get_serializable(query);

    info!("Claim serializable: {:?}", claim_serializable);

    let block = eth_client.get_block(query.height).await?;

    info!("Generating proof for query with id: {:?}", query_id);
    // Generate proof
    let query_prover =
        match proof::run_cairo_verifier(claim_serializable, &attestation_fragment, block).await {
            Ok(cairo_output) => {
                info!("Generated proof for query with id: {:?}", query_id);
                cairo_output
            }
            Err(e) => {
                info!(
                    "Failed to run cairo verifier for query with id: {:?}, error: {:?}",
                    query_id, e
                );
                return Err(Error::Proof);
            }
        };

    if stone_proof {
        let result = proof::cairo_generate_proof(query_prover, stone_proof, stone_proof)
            .await
            .map_err(|e| {
                error!(
                    "Failed to generate proof for query with id: {:?}, error: {:?}",
                    query_id, e
                );
                Error::Proof
            })?;

        match result {
            either::Left((mut stone_proof, _stone_proof_dir)) => {
                // Strip off annotations, prover config, and private input
                stone_proof
                    .strip_off_annotations()
                    .strip_off_prover_config()
                    .strip_off_private_input();

                // json serialize proof
                let proof_json = serde_json::to_string_pretty(&stone_proof.proof())?;

                Ok(Either::Left(proof_json.as_bytes().to_vec()))
            }
            either::Right(_claim_files) => {
                warn!("We shouldn't really fall in this code path");
                Err(Error::Proof)
            }
        }
    } else {
        let stone_prover_input_files = query_prover
            .get_claim_files()
            .map_err(|_e| Error::QueryFiles)?;

        Ok(Either::Right(stone_prover_input_files))
    }
}

fn get_serializable(query: &Query) -> ClaimSerializable {
    ClaimSerializable {
        id: ClaimIdentifier::new(query.height, query.index),

        felt_ranges: query
            .layout_segments
            .iter()
            .map(|layout| Range {
                start: usize::try_from(layout.offset).expect("layout offset is too large"),
                end: usize::try_from(layout.offset + layout.size).expect("layout end is too large"),
            })
            .collect::<Vec<_>>(),
    }
}
