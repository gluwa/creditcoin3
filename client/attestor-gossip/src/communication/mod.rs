use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use thiserror::Error;

use attestation_chain::attestation_fragment::AttestationFragmentSerializable;
use attestor_primitives::bls::{Bls, CryptoScheme};
use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestorId, ChainKey, Digest, PalletDigest, Round,
};

pub mod gossip;
pub mod validator;

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation<H, AccountId> {
    pub attestation_data: AttestationPrimitive<H>,
    pub attestor: AccountId,
    pub proof_of_inclusion: vrf::ProofOfInclusion,
    pub signature: sp_core::sr25519::Signature,
    pub signature_bls: <Bls as CryptoScheme>::Signature,
    pub continuity_proof: AttestationFragmentSerializable,
}

impl<H, AccountId> Attestation<H, AccountId>
where
    H: AsRef<[u8]>,
    AccountId: Into<[u8; 32]> + Clone,
{
    pub fn digest(&self) -> PalletDigest {
        self.attestation_data.digest()
    }

    pub fn round(&self) -> Round {
        self.attestation_data.round()
    }

    pub fn chain_key(&self) -> ChainKey {
        self.attestation_data.chain_key()
    }

    pub fn header_number(&self) -> u64 {
        self.attestation_data.header_number
    }

    pub fn attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.attestor.clone().into())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Gossip engine exited")]
    GossipEngineExited,
    #[error("Invalid attestation signature")]
    InvalidAttestationDataSignature,
    #[error("Invalid attestation vrf output")]
    InvalidAttestationVrfOuput,
    #[error("Attestation to old")]
    AttestationTooOld,
    #[error("Attestation to early")]
    AttestationTooEarly,
    #[error("Attestation header number invalid")]
    AttestationHeaderNumberInvalid,
    #[error("Attestation already exists")]
    AttestationExists,
    #[error("Error creating inherent data")]
    ErrorCreatingInherent,
    #[error("Sender {0:?} is not an attestor")]
    NotAnAttestor(AttestorId),
    #[error("Digest missmatch")]
    DigestMissMatch,
    #[error("Failed to fetch last digest")]
    FetchLastDigestError,
    #[error("Invalid bls signature")]
    InvalidBlsSignature,
    #[error("Invalid sr signature")]
    InvalidSrSignature,
    #[error("Chain not supported")]
    ChainNotSupported,
    #[error("Sp api error")]
    SpApiError(#[from] sp_api::ApiError),
    #[error("Vrf error")]
    VrfError(#[from] vrf::Error),
    #[error("Attestor {0:?} not eligible")]
    AttestorNotEligible(AttestorId),
    #[error("Attestor {0:?} not active")]
    AttestorNotActive(AttestorId),
    #[error("Failed to get attestation interval")]
    FailedToGetAttestationInterval,
    #[error("Failed to get last attestation after existance confirmed")]
    FailedToGetLastAttestation,
    #[error("Failed to get round configuration")]
    RoundConfigNotSet,
    #[error("Other error: {0}")]
    Other(String),
    #[error("Finality stream terminated")]
    FinalityStreamTerminated,
    #[error("Attestation data contains invalid epoch")]
    InvalidEpoch,
    #[error("Attestation contains an invalid continuity proof")]
    InvalidAttestationContinuityProof,
    #[error("Attestation contains an invalid epoch mismatch")]
    EpochMismatch,
    #[error("Worker is syncing")]
    WorkerInSync,
    #[error("Overflow error")]
    Overflow,
    #[error("Attestation round already concluded")]
    RoundAlreadyConcluded,
    #[error("Attestation round config not found")]
    RoundConfigNotFound,
}
