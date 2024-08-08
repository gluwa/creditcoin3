use anyhow::Result;
use std::ops::Range;
use tracing::info;

use eth::{transaction::BlockItem, Client};
use proof::cairo_generate_proof;
use prover_primitives::claim::{ClaimIdentifier, ClaimKind, ClaimSerializable};
use utils::block_item_traits::BlockItemIdentifier;

use crate::{fragment, AttestationCacheType, CcClientArc, Claim};

const SCRIPT_SOURCE: &str = "../cairo/scripts/verify_merkle_proof.sh";

// Process a claim
// Parameters:
// - `cc3_client`: cc3 client
// - `eth_client`: eth client that is initialized with the rpc url of the chain the claim is from
// - `claim`: claim to process
// - `attestation_cache`: attestation cache
pub async fn process(
    cc3_client: CcClientArc,
    eth_client: Client,
    claim: Claim,
    attestation_cache: &AttestationCacheType,
) -> Result<()> {
    info!("Processing claim with hash: {:?}", claim.hash);

    // Get the attestation fragment
    let attestation_fragment =
        fragment::get_for_claim(&cc3_client, eth_client.clone(), &claim, attestation_cache).await?;

    info!(
        "Got attestation fragment for claim with hash: {:?}",
        claim.hash
    );

    // Format claim to ClaimSerializable
    let claim_kind = match claim.claim.id.kind {
        cc_client::cc3::runtime_types::pallet_prover::types::ClaimKind::Tx => ClaimKind::Tx,
        cc_client::cc3::runtime_types::pallet_prover::types::ClaimKind::Rx => ClaimKind::Rx,
    };
    let claim_serializable = ClaimSerializable {
        id: ClaimIdentifier {
            kind: claim_kind,
            block_item_id: BlockItemIdentifier::new(
                claim.claim.id.block_item_id.block_number.into(),
                u64::from(claim.claim.id.block_item_id.index),
            ),
        },
        felt_ranges: claim
            .claim
            .felt_ranges
            .into_iter()
            .map(|f| Range {
                start: f.start as usize,
                end: f.end as usize,
            })
            .collect(),
    };

    info!("Claim serializable: {:?}", claim_serializable);

    // Convert transactions to bytes
    let tx = eth_client
        .get_transactions(claim.claim.id.block_item_id.block_number)
        .await?;
    let tx_bytes = tx
        .iter()
        .map(eth::transaction::Transaction::to_bytes)
        .collect::<Vec<_>>();

    // Convert receipts to bytes
    let rx = eth_client
        .get_receipts(claim.claim.id.block_item_id.block_number)
        .await?;
    let rx_bytes = rx
        .iter()
        .map(eth::transaction::Receipt::to_bytes)
        .collect::<Vec<_>>();

    info!("Generating proof for claim with hash: {:?}", claim.hash);
    // Generate proof
    let cairo_output_of_stone_proof = match cairo_generate_proof(
        claim_serializable,
        &attestation_fragment,
        tx_bytes,
        rx_bytes,
        true,
        false,
    )
    .await
    {
        Ok(cairo_output) => {
            info!("Generated proof for claim with hash: {:?}", claim.hash);
            cairo_output
        }
        Err(e) => {
            info!(
                "Failed to generate proof for claim with hash: {:?}, error: {:?}",
                claim.hash, e
            );
            return Ok(());
        }
    };

    info!("Submitting proof for claim with hash: {:?}", claim.hash);
    // TODO: what is this either left or right
    match cairo_output_of_stone_proof {
        either::Left((mut stone_proof, stone_proof_dir)) => {
            proof::run_stone_verify_script(SCRIPT_SOURCE, &stone_proof_dir)
                .await
                .unwrap();
            stone_proof
                .strip_off_annotations()
                .strip_off_prover_config()
                .strip_off_private_input();

            let proof_json = serde_json::to_string_pretty(&stone_proof.proof())?;

            // TODO: write to path that is configurable
            // write to file
            tokio::fs::write(
                format!("claims/{}-proof.json", claim.hash),
                proof_json.clone(),
            )
            .await?;

            // Submit result to cc3
            cc3_client
                .submit_proof(claim.hash, proof_json.as_bytes().to_vec())
                .await?;

            info!("Submitted proof for claim with hash: {:?}", claim.hash);
        }
        // Ignore this case since we are always running cairo proof mode
        // TODO, refactor lib to be able to remove this case
        either::Right(_cairo_output) => return Ok(()),
    };

    Ok(())
}
