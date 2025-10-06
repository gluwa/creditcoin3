use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use thiserror::Error;

use attestor_primitives::{
    attestation_fragment::AttestationFragmentSerializable,
    bls::{Bls, CryptoScheme},
    Attestation as AttestationPrimitive, AttestorId, ChainKey, Digest, Round,
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
    pub epoch: u64,
}

impl<H, AccountId> Attestation<H, AccountId>
where
    H: AsRef<[u8]>,
    AccountId: Into<[u8; 32]> + Clone,
{
    pub fn digest(&self) -> Digest {
        self.attestation_data.digest()
    }

    pub fn prev_digest(&self) -> Option<Digest> {
        self.attestation_data.prev_digest()
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

// cost scalars for reporting peers.
mod cost {
    use sc_network::ReputationChange as Rep;
    // Message that's for an outdated round.
    pub(super) const OUTDATED_MESSAGE: Rep = Rep::new(-50, "ATTESTOR: Past message");
    // Message that's from the future relative to our current set-id.
    pub(super) const FUTURE_MESSAGE: Rep = Rep::new(-100, "ATTESTOR: Future message");
    // Vote message containing bad signature.
    pub(super) const BAD_SIGNATURE: Rep = Rep::new(-100, "ATTESTOR: Bad signature");
    // Message received with vote from voter not in validator set.
    pub(super) const UNKNOWN_VOTER: Rep = Rep::new(-150, "ATTESTOR: Unknown voter");
    // Reputation cost per byte for un-decodable message.
    pub(super) const PER_UNDECODABLE_BYTE: i32 = -5;
}

// benefit scalars for reporting peers.
mod benefit {
    use sc_network::ReputationChange as Rep;
    pub(super) const VOTE_MESSAGE: Rep = Rep::new(100, "ATTESTOR: Round vote message");
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
    #[error("Double vote detected")]
    DoubleVote,
    #[error("Stale vote detected")]
    StaleVote,
}
