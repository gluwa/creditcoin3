use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

use attestation_chain::attestation_fragment::AttestationFragmentSerializable;
use attestor_primitives::bls::{Bls, CryptoScheme};
use attestor_primitives::{Attestation as AttestationPrimitive, AttestorId, Digest, Round};

pub mod gossip;

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation<H, AccountId> {
    pub attestation_data: AttestationPrimitive<H>,
    pub attestor: AccountId,
    pub proof_of_inclusion: vrf::ProofOfInclusion,
    pub signature: sp_core::sr25519::Signature,
    pub signature_bls: <Bls as CryptoScheme>::Signature,
    pub continuity_proof: Vec<AttestationFragmentSerializable>,
}

impl<H, AccountId> Attestation<H, AccountId>
where
    H: AsRef<[u8]>,
    AccountId: Into<[u8; 32]> + Clone,
{
    pub fn digest(&self) -> Digest {
        self.attestation_data.digest()
    }

    pub fn round(&self) -> Round {
        self.attestation_data.round()
    }

    pub fn attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.attestor.clone().into())
    }
}
