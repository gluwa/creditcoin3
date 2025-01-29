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

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(5);

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
) -> Result<Either<Proof, Vec<PathBuf>>, Error> {
    let query_id = query.id();
    info!("Processing query with id: {:?}", query_id);

    let mut retry_count = 0;

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
                    query_id, last_height, query_height, retry_count, MAX_RETRIES, RETRY_DELAY
                );
                tokio::time::sleep(RETRY_DELAY).await;
            }
            Err(e) => return Err(e.into()),
        }
    };

    info!("Got attestation fragment for query with id: {:?}", query_id);

    // Format claim to ClaimSerializable
    let claim_serializable = ClaimSerializable {
        id: ClaimIdentifier::new(query.height, query.index),

        felt_ranges: query
            .layout_segments
            .iter()
            .map(|layout| Range {
                start: usize::try_from(layout.offset).expect("layout offset is too large"),
                end: usize::try_from(layout.offset + layout.size).expect("layout end is too large"),
            })
            .collect::<Vec<_>>(),
    };

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
