use crate::print_with_timestamp;
use anyhow::anyhow;
use attestation_chain::attestation_checkpoints::{AttestationCheckpoint, AttestationCheckpoints};
use attestation_chain::attestation_fragment::AttestationFragment;
use colored::Colorize;
use either::Either;
use proof::claim_prover::{build_prover, ClaimProver};
use proof::types::CairoVerifierOutput;
use proof::types::ClaimProverError;
use proof::types::StoneProof;
use prover_primitives::claim::ClaimSerializable;

pub async fn cairo_verify_claim(
    url: &str,
    claim: ClaimSerializable,
    claim_attestation_fragment: &AttestationFragment,
    checkpoints: &AttestationCheckpoints,
    cairo_proof_mode: bool,
    force_stone_proving: bool,
) -> anyhow::Result<Either<StoneProof, CairoVerifierOutput>> {
    //) -> anyhow::Result<Option<StoneProof>> {
    let block_number = claim.id().block_item_id.block_number();
    let claim_checkpoint = checkpoints.checkpoint_for(block_number).ok_or(anyhow!(
        "claim block number {} matches no checkpoints",
        block_number
    ))?;

    let claim_attestation_slice = claim_attestation_fragment
        .attestation_slice_for(block_number, Some(claim_checkpoint.n()))
        .ok_or(anyhow!("unable to slice fragment {claim_attestation_fragment:?} for block number {} and checkpoint {}", block_number, claim_checkpoint.n()))?;

    println!("\n");
    print_with_timestamp("---------- cairo claim proving task is starting ----------".bold());
    println!("claim: {:?}", claim);
    println!("fetching block and building merkle trees...");

    let mut cairo_verifier = build_prover(url, claim.clone(), claim_attestation_slice)
        .await
        .map(|claim_cairo_verifier| {
            print_with_timestamp("done".into());
            println!("\ncairo0 input file {}", format!("{:?}", claim_cairo_verifier.file_name()).bright_cyan());
            println!("running script {}", format!("{:?}", ClaimProver::script_source()).bright_cyan());

            claim_cairo_verifier
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
        .cairo_verify(cairo_proof_mode)
        .await
        .map_err(|err| anyhow!("{err:?}"))?;

    let output = cairo_verifier
        .cairo_output()
        .ok_or(anyhow!("successful verification expected to yield output"))?;
    print_with_timestamp("----- cairo verification successful -----".green());
    println!("cairo verification output:");
    println!("{}", format!("{:?}", output).bold());

    let output_checkpoint = AttestationCheckpoint::try_from_block(
        output.continuity_checkpoint_block_number,
        output.continuity_checkpoint_digest,
    )
    .ok_or(anyhow!(
        "expected to get a valid checkpoint from cairo verifier's output"
    ))?;

    if checkpoints.verify_claim_continuity(&output_checkpoint) {
        println!(
            "{}",
            format!(
                "\nclaim continuity validated at checkpoint: {:?}",
                output_checkpoint
            )
            .green()
        );
    } else {
        return Err(anyhow!(
            "claim continuity not validated on attestation chain"
        ));
    };

    if cairo_proof_mode {
        println!("running stone-prover, will take a while...");

        cairo_verifier
            .stone_prove(force_stone_proving)
            .await
            .map(|msg| {
                println!("{msg}");
            })
            .map_err(|err| anyhow!("{err:?}"))?;

        cairo_verifier.stone_proof().map(Either::<_, _>::Left)
    } else {
        Ok(Either::Right(output.clone()))
    }
}
