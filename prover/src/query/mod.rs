use anyhow::Result;
use either::Either;
use eth::Client;
use sp_core::H256;
use std::path::PathBuf;
use thiserror::Error;
use tokio_retry::strategy::{jitter, FibonacciBackoff};
use tokio_retry::Retry;
use tracing::{error, info, warn};

use pallet_prover_primitives::Query;
use prover_primitives::claim::{ClaimIdentifier, ClaimSerializable};

use crate::postgres::queryfragmenttype::NewQueryFragmentType;
use crate::query::Error::AttestationCacheError;
use crate::{attestation::fragment, AttestationCacheType};

pub mod external;

/// Proof as bytes
pub type Proof = Vec<u8>;

/// Query id
pub type QueryId = H256;

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
    #[error("Attestation cache error")]
    AttestationCacheError,
}

// Process a query
// Parameters:
// - `cc3_client`: cc3 client
// - `eth_client`: eth client that is initialized with the rpc url of the chain the claim is from
// - `query`: query to process
// - `attestation_cache`: attestation cache
// - `stone_proof`: whether to generate a stone proof
pub async fn process(
    eth_client: Client,
    query: &Query,
    attestation_cache: &AttestationCacheType,
    stone_proof: bool,
) -> Result<Either<Proof, Vec<PathBuf>>, Error> {
    let query_id = query.id();
    info!("Processing query with id: {:?}", query_id);

    // Retry strategy with Fibonacci backoff and jitter (1, 1, 2, 3, 5, ...)
    let retry_strategy = FibonacciBackoff::from_millis(1000).map(jitter).take(5);

    let fragment_result = Retry::spawn(retry_strategy.clone(), || {
        fragment::get_for_claim(&eth_client, query, attestation_cache)
    })
    .await;

    // Get the attestation fragment
    let attestation_fragment = match fragment_result {
        Ok(fragment) => fragment,
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
        Err(e) => {
            // This path is taken for any other error after all retries have failed.
            error!(
                "Failed to get attestation fragment for query {:?} after multiple retries: {:?}",
                query_id, e
            );
            return Err(e.into());
        }
    };

    info!("Got attestation fragment for query with id: {:?}", query_id);

    let claim_serializable = get_serializable(query);

    info!("Claim serializable: {:?}", claim_serializable);

    let block = Retry::spawn(retry_strategy, || eth_client.get_block(query.height))
        .await
        .map_err(|e| {
            error!(
                "Failed to get block {} after multiple retries: {:?}",
                query.height, e
            );
            e
        })?;

    // Check if we need to force stone proving based on fragment type changes
    let maybe_force_stone_proving =
        check_and_update_fragment_type(query, query_id, attestation_cache).await?;

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
        let result =
            proof::cairo_generate_proof(query_prover, stone_proof, maybe_force_stone_proving)
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

// Checks if the fragment type for a query has changed and updates the database.
// Returns whether stone proving should be forced based on fragment type changes.
async fn check_and_update_fragment_type(
    query: &Query,
    query_id: QueryId,
    attestation_cache: &AttestationCacheType,
) -> Result<bool, Error> {
    // Check the current fragment type for this query
    let current_fragment_type = fragment::get_fragment_type_for_query(query, attestation_cache)
        .await
        .map_err(|_e| AttestationCacheError)?;

    let query_id_str = format!("{query_id:x}");

    // Check if we have previously processed this query and if the fragment type has changed
    let force_due_to_fragment_change = if let Some(stored_fragment_type) = attestation_cache
        .get_query_fragment_type_by_id(query_id_str.clone())
        .await
        .map_err(|_e| AttestationCacheError)?
    {
        let stored_type = stored_fragment_type
            .fragment_type
            .parse::<fragment::FragmentType>()
            .map_err(|_e| AttestationCacheError)?;

        if stored_type == current_fragment_type {
            info!(
                "Fragment type unchanged for query {:?}: {}",
                query_id, current_fragment_type
            );
            false
        } else {
            info!(
                "Fragment type changed for query {:?}: {} -> {}, forcing stone proving",
                query_id, stored_type, current_fragment_type
            );
            true
        }
    } else {
        info!(
            "First time processing query {:?} with fragment type: {}",
            query_id, current_fragment_type
        );
        true
    };

    // Store/update the current fragment type for this query
    let new_query_fragment_type = NewQueryFragmentType::new(
        query_id_str,
        query.chain_id,
        query.height,
        current_fragment_type.to_string(),
    );

    attestation_cache
        .upsert_query_fragment_type(new_query_fragment_type)
        .await
        .map_err(|e| {
            error!("Database error during query fragment type upsert: {:?}", e);
            AttestationCacheError
        })?;

    Ok(force_due_to_fragment_change)
}

fn get_serializable(query: &Query) -> ClaimSerializable {
    // Convert byte segments into felt ranges expected by Cairo program
    let felt_ranges =
        prover_primitives::claim::byte_segments_into_felt_ranges(&query.layout_segments);

    ClaimSerializable {
        id: ClaimIdentifier::new(query.height, query.index),
        // Ranges should already come to us compacted and sorted, but we enforce this here
        felt_ranges: prover_primitives::claim::compact_and_sort_ranges(felt_ranges),
        query_id: query.id(),
        chain_id: query.chain_id,
    }
}
