mod claim_prover;
mod eth;
mod fragment;
mod types;

use anyhow::anyhow;
use claim_prover::build_prover;

use crate::fragment::AttestationFragment;
use prover_primitives::claim::{Claim, ClaimKind};
use std::fmt::Debug;

const SOME_FRAGMENT_SIZE: usize = 5;

pub fn cairo_generate_proof<H, Address, A>(
    url: &str,
    claim: Claim<Address>,
    attestation_fragment: &AttestationFragment<H, A>,
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

    let prover = build_prover(url, claim, attestation_chain_slice);
    //verifier.cairo_verify(false)?;
    Ok(())
}
