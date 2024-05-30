use crate::fragment::FragmentSlice;
use prover_primitives::claim::{Claim, ClaimKind};

pub struct CairoClaimVerifier {}

pub struct CairoVerifierError {}

pub async fn build_verifier<'a, H: Clone, A, Address>(
    url: &str,
    claim: Claim<Address>,
    attestation_chain_slice: FragmentSlice<'a, H, A>,
) -> Result<CairoClaimVerifier, CairoVerifierError> {
    Ok(CairoClaimVerifier {})
}
