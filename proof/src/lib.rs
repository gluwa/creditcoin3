use anyhow::{anyhow, Result};
use attestation_chain::AttestationChainParams;
use colored::Colorize;
use either::Either;
use eth_common::OrderedBlock;
use tracing::{debug, info};

use attestation_chain::attestation_checkpoints::AttestationCheckpoint;
use attestation_chain::attestation_fragment::{
    AttestationFragment, FragmentContinuityBlocksSerializable,
};
use prover_primitives::claim::ClaimSerializable;
use prover_primitives::types::{CairoVerifierOutput, ClaimProverError, StoneProof};

use crate::claim_prover::{build_prover, ClaimProver};

pub mod claim_prover;
pub mod json_serializable;

pub async fn cairo_generate_proof(
    cairo_verifier: ClaimProver,
    stone_proof: bool,
    force_stone_proving: bool,
) -> Result<Either<(StoneProof, String), CairoVerifierOutput>> {
    if stone_proof {
        info!("running stone-prover, will take a while...");

        cairo_verifier
            .stone_prove(force_stone_proving)
            .await
            .map(|msg| {
                info!("{}", msg);
            })
            .map_err(|err| anyhow!("{err:?}"))?;

        cairo_verifier
            .stone_proof()
            .map(|stone_proof| Either::Left((stone_proof, cairo_verifier.default_dir())))
    } else {
        Ok(Either::Right(
            cairo_verifier
                .cairo_output()
                .ok_or(anyhow!("successful verification expected to yield output"))?
                .clone(),
        ))
    }
}

pub async fn run_cairo_verifier(
    claim: ClaimSerializable,
    claim_attestation_fragment: &AttestationFragment,
    block: OrderedBlock,
) -> Result<ClaimProver> {
    debug!("\n");
    info!("---------- cairo claim proving task is starting ----------");
    debug!("claim: {:?}", claim);

    let claim_block_number = claim.id().block_number();
    let fragment_subset = claim_attestation_fragment
        .blocks_serializable(claim_block_number)
        .map_err(|e| anyhow!("{:?}", e))?;
    debug!("fetching block and building merkle trees...");

    let fragment_continuity_blocks = FragmentContinuityBlocksSerializable::from(fragment_subset);

    let mut cairo_verifier = build_prover(claim.clone(), fragment_continuity_blocks, block)
        .await
        .inspect(|claim_cairo_verifier| {
            debug!("done");
            debug!("\ncairo0 input file {}", format!("{:?}", claim_cairo_verifier.file_name()).bright_cyan());
            debug!("running script {}", format!("{:?}", ClaimProver::verify_merkle_command()).bright_cyan());
        })
        .map_err(|err| {
            anyhow!("{}",
                match &err {
                    ClaimProverError::AttestationFragmentMismatch(b, tail, head) =>
                        format!("can't create attestation checkpoint slice for {b} on this attestation chain ({tail:?}, {head:?})"),
                    ClaimProverError::BlockFetchFailure(msg) =>
                        format!("failure while fetching block corresponding claim: {msg}"),
                    ClaimProverError::ClaimNotIdentified =>
                        format!("claim {claim:?} was not identified in the block"),
                    err => format!("could not build verifier: {err:?}"),
                }
            )
        })?;

    cairo_verifier
        // default to true for now
        .cairo_verify(true)
        .await
        .map_err(|err| anyhow!("{err:?}"))?;

    let output = cairo_verifier
        .cairo_output()
        .ok_or(anyhow!("successful verification expected to yield output"))?;

    info!("----- cairo verification successful -----");
    debug!("cairo verification output:");
    debug!("{}", format!("{:?}", output).bold());

    let input_checkpoint = claim_attestation_fragment
        .checkpoint()
        .expect("attestation fragment expected to be full");

    let output_checkpoint = AttestationCheckpoint::try_from_block(
        // TODO: The use of AttestationChainParams is fully vestigial here since
        // only the genesis field is used, and the current consensus is to
        // always use the gnensis block 0. Rework once outdated crates sharing
        // `try_from_block` are removed.
        AttestationChainParams::new(0, 10),
        claim_block_number - 1 + output.continuity_proof_length - 1,
        output.continuity_checkpoint_digest,
    )
    .ok_or(anyhow!(
        "expected to get a valid checkpoint from cairo verifier's output"
    ))?;

    if input_checkpoint == output_checkpoint {
        debug!(
            "{}",
            format!(
                "\nclaim continuity validated at checkpoint: {:?}",
                output_checkpoint
            )
            .green()
        );
    } else {
        return Err(anyhow!(
            "claim continuity not validated on attestation chain, here {:?}, there {:?}",
            input_checkpoint,
            output_checkpoint
        ));
    };

    Ok(cairo_verifier)
}
