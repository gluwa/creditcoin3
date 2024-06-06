pub mod claim_prover;
pub mod eth;
pub mod fragment;
pub mod types;

use anyhow::anyhow;
use attestor::cc3::cc3::tx;
use claim_prover::build_prover;

use crate::fragment::AttestationFragment;
use crate::types::ClaimProverError;
use prover_primitives::claim::Claim;

const SOME_FRAGMENT_SIZE: usize = 5;

pub async fn cairo_generate_proof<H, Address, A>(
    url: &str,
    claim: Claim<Address>,
    attestation_fragment: &AttestationFragment<H, A>,
    tx_bytes: Vec<Vec<u8>>,
    rx_bytes: Vec<Vec<u8>>,
) -> anyhow::Result<()>
where
    H: Clone,
{
    let claim_block_number: u64 = claim.block_number;
    let attestation_chain_slice = attestation_fragment.attestation_slice_for(claim_block_number, None)
        .ok_or(anyhow!("can't create attestation checkpoint slice for {} on this attestation chain ({:?}, {:?})",
            claim_block_number,
            attestation_fragment.tail().map(|att| att.header_number()),
            attestation_fragment.head().map(|att| att.header_number())))?;

    let prover = build_prover(url, claim, attestation_chain_slice, tx_bytes, rx_bytes)
        .await
        .and_then(|claim_prover| {
            println!("done");
            println!("cairo0 input file {}", format!("{:?}", claim_prover.file_name()));
            Ok(claim_prover)
    })
        .map_err(|err| {
            anyhow!("{}",
                match &err {
                    ClaimProverError::AttestationFragmentMismatch(b, tail, head) =>
                        format!("can't create attestation checkpoint slice for {b} on this attestation chain ({tail:?}, {head:?})"),
                    ClaimProverError::BlockFetchFailure(msg) =>
                        format!("failure while fetching block corresponding claim: {msg}"),
                    ClaimProverError::ClaimNotIdentified =>
                        "claim was not identified in the block".to_string(),
                    ClaimProverError::ClaimNotUnique =>
                        "claim not uniquely identified in the block, refine filtering criteria".to_string(),
                    err => format!("could not build verifier: {err:?}"),
                }
            )
        })?
        .cairo_verify(true)
        .await
        .map_err(|err| anyhow!("{err:?}"))
        .map(|mut claim_prover| {
            let output = claim_prover.take_output().expect("successful verification yields output");

            println!("----- cairo verification successful -----");
            println!("cairo verification output: {:?}", output);
            claim_prover
        })
        // ToDo continuity validation at checkpoints here
        .and_then(|claim_prover|{
            if true {
                Ok(claim_prover)
            } else {
                Err(anyhow!("proof generation failed"))
            }
        })?;

    // ToDo always stone proving, make this configurable
    if true {
        println!("stone proving... will take some time");

        prover
            .stone_prove(true)
            .await
            .map(|msg| println!("{}", msg))
            .map_err(|err| anyhow!("{err:?}"))
    } else {
        Ok(())
    }
}
