use anyhow::{anyhow, Result};
use colored::Colorize;
use either::Either;
use eth_common::OrderedBlock;
use tracing::{debug, info};

use attestor_primitives::attestation_fragment::{
    AttestationFragment, FragmentContinuityBlocksSerializable,
};
use prover_primitives::query::QuerySerializable;
use prover_primitives::types::{CairoVerifierOutput, QueryProverError, StoneProof};

use crate::query_prover::{build_prover, QueryProver};

pub mod json_serializable;
pub mod query_prover;

pub async fn cairo_generate_proof(
    cairo_verifier: QueryProver,
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
    query: QuerySerializable,
    query_attestation_fragment: &AttestationFragment,
    block: OrderedBlock,
) -> Result<QueryProver> {
    debug!("\n");
    info!("---------- cairo query proving task is starting ----------");
    debug!("query: {:?}", query);

    let query_block_number = query.id().block_number();
    let fragment_subset = query_attestation_fragment
        .blocks_serializable(query_block_number)
        .map_err(|e| anyhow!("{:?}", e))?;
    debug!("fetching block and building merkle trees...");

    let fragment_continuity_blocks = FragmentContinuityBlocksSerializable::from(fragment_subset);

    let mut cairo_verifier = build_prover(query.clone(), fragment_continuity_blocks, block)
        .await
        .inspect(|query_cairo_verifier| {
            debug!("done");
            debug!("\ncairo0 input file {}", format!("{:?}", query_cairo_verifier.file_name()).bright_cyan());
            debug!("running script {}", format!("{:?}", QueryProver::verify_merkle_command()).bright_cyan());
        })
        .map_err(|err| {
            anyhow!("{}",
                match &err {
                    QueryProverError::AttestationFragmentMismatch(b, tail, head) =>
                        format!("can't create attestation checkpoint slice for {b} on this attestation chain ({tail:?}, {head:?})"),
                    QueryProverError::BlockFetchFailure(msg) =>
                        format!("failure while fetching block corresponding query: {msg}"),
                    QueryProverError::QueryNotIdentified =>
                        format!("query {query:?} was not identified in the block"),
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
    debug!("{}", format!("{output:?}").bold());

    Ok(cairo_verifier)
}
