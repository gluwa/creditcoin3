use anyhow::Result;
use std::ops::Range;
use tracing::info;

use attestation_chain::{attestation_fragment::AttestationFragment, AttestationChainParams};
use pallet_prover_primitives::Query;
use proof::cairo_generate_proof;
use prover_primitives::claim::{ClaimIdentifier, ClaimSerializable};

use crate::{contract, fragment, AttestationCacheType, CcClientArc, EthClientArc};

// Process a claim
// Parameters:
// - `cc3_client`: cc3 client
// - `eth_client`: eth client that is initialized with the rpc url of the chain the claim is from
// - `query`: query to process
// - `attestation_cache`: attestation cache
pub async fn process(
    cc3_client: CcClientArc,
    eth_client: EthClientArc,
    query: &Query,
    attestation_cache: &AttestationCacheType,
) -> Result<Vec<u8>> {
    let query_id = query.id();
    info!("Processing query with id: {:?}", query_id);

    let interval = cc3_client
        .get_attestation_chain_interval(query.chain_id)
        .await?
        .unwrap_or(0);
    info!("Got attestation chain interval: {:?}", interval);

    // Get the attestation fragment
    let attestation_fragment: AttestationFragment =
        fragment::get_for_claim(&eth_client, query, attestation_cache, interval).await?;

    info!("Got attestation fragment for query with id: {:?}", query_id);

    // Format claim to ClaimSerializable
    let claim_serializable = ClaimSerializable {
        id: ClaimIdentifier::new(query.height, query.index),

        felt_ranges: query
            .layout_segments
            .iter()
            .map(|layout| Range {
                start: usize::try_from(layout.offset).expect("layout offset is too large"),
                end: usize::try_from(layout.size).expect("layout end is too large"),
            })
            .collect::<Vec<_>>(),
    };

    info!("Claim serializable: {:?}", claim_serializable);

    let block = eth_client.get_block(query.height).await?;

    info!("Generating proof for query with id: {:?}", query_id);
    // Generate proof
    let cairo_output_of_stone_proof = match cairo_generate_proof(
        AttestationChainParams::new(
            0,
            interval
                .try_into()
                .map_err(|_| anyhow::anyhow!("Failed to convert interval to u32"))?,
        ),
        claim_serializable,
        &attestation_fragment,
        block,
        true,
        false,
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
            stone_proof
                .strip_off_annotations()
                .strip_off_prover_config()
                .strip_off_private_input();

            let proof_json = serde_json::to_string_pretty(&stone_proof.proof())?;

            Ok(proof_json.as_bytes().to_vec())
        }
        // Ignore this case since we are always running cairo proof mode
        // TODO, refactor lib to be able to remove this case
        either::Right(_cairo_output) => Err(anyhow::anyhow!("Cairo output is not stone")),
    }
}

pub async fn _dummy_process(
    _cc3_client: CcClientArc,
    eth_client: EthClientArc,
    query: Query,
    _attestation_cache: &AttestationCacheType,
) -> Result<()> {
    let query_id = query.id();
    info!("Processing query with id: {:?}", query_id);

    let current_dir = std::env::current_dir()?;
    let proof_example_path = if current_dir.ends_with("creditcoin3-next") {
        "cairo/stone-verifier/proof_example.json"
    } else {
        "proof_example.json"
    };

    let proof_example = tokio::fs::read(proof_example_path).await?;
    info!(
        "Submitting proof for query with id: {:?}, proof len {}",
        query_id,
        proof_example.len()
    );
    // Submit result to prover contract
    let tx_hash = contract::submit_proof(&eth_client, query, proof_example).await?;
    info!(
        "Submitted proof for query with id: {:?}, tx hash: {:?}",
        query_id, tx_hash
    );

    Ok(())
}
