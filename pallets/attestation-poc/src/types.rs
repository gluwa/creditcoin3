use attestor_primitives::SignedAttestation;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::hash::H256;
use sp_core::RuntimeDebug;

pub type BlockNumber = u64;
pub type Digest = H256;

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct Attestation<H, A>
where
    H: Clone,
{
    pub attestation: SignedAttestation<H, A>,
    pub prev_digest: Digest,
}

impl<H, A> Attestation<H, A>
where
    H: Clone,
{
    pub fn new(input: SignedAttestation<H, A>, prev_digest: Digest) -> Self {
        Self {
            attestation: input,
            prev_digest,
        }
    }
}
