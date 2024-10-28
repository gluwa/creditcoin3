use anyhow::Result;
use either::Either;
use std::ops::Range;
use tracing::info;

use attestation_chain::attestation_fragment::AttestationFragment;
use pallet_prover_primitives::Query;
use proof::cairo_generate_proof;
use prover_primitives::{
    claim::{ClaimIdentifier, ClaimSerializable},
    types::StoneProofPublicInput,
};

use crate::{attestation::fragment, AttestationCacheType, EthClientArc};

/// Proof as bytes
pub type Proof = Vec<u8>;

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
) -> Result<Either<Proof, StoneProofPublicInput>> {
    let query_id = query.id();
    info!("Processing query with id: {:?}", query_id);

    // Get the attestation fragment
    let attestation_fragment: AttestationFragment =
        fragment::get_for_claim(&eth_client, query, attestation_cache).await?;

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
    let cairo_output_of_stone_proof = match cairo_generate_proof(
        claim_serializable,
        &attestation_fragment,
        block,
        true,
        stone_proof,
    )
    .await
    {
        Ok(cairo_output) => {
            info!("Generated proof for query with id: {:?}", query_id);
            cairo_output
        }
        Err(e) => {
            info!(
                "Failed to generate proof for query with id: {:?}, error: {:?}",
                query_id, e
            );
            return Err(anyhow::anyhow!("Failed to generate proof"));
        }
    };

    // info!("Submitting proof for claim with hash: {:?}", claim.hash);
    // TODO: what is this either left or right
    match cairo_output_of_stone_proof {
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
        either::Right(cairo_output) => Ok(Either::Right(cairo_output)),
    }
}
