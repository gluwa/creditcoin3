//! New lightweight vote helpers.
//!
//! The protocol change from v1 → v2:
//!
//! - v1 votes carried a full `Attestation` (`attestation_data + signature_bls + continuity_proof`).
//! - v2 votes carry only `(chain_key, height, digest, attestor_id, signature_bls)`. The signed
//!   bytes are still `AttestationData.serialize()` — that means a verifier must reconstruct the
//!   *same* AttestationData locally to verify a remote vote. In practice each attestor produces
//!   its own local AttestationData at each height (via the eth `StreamAttestation`), so the
//!   verifier just looks up its local AttestationData at the matching height.
//!
//! If a vote arrives at height `H` and the verifier has not yet produced its own local
//! AttestationData at `H`, the vote is dropped (it will be re-gossiped on the next heartbeat —
//! libp2p gossipsub already handles this). This is a deliberate trade: we save bandwidth by
//! avoiding redundant proof transport at the cost of needing one local production round before
//! we accept votes at that height.

use attestor_primitives::{AttestorId, ChainKey, Digest, Height};

pub use attestor_pool::Vote;

/// Result of verifying a remote vote against the local AttestationData at the same height.
#[derive(Debug)]
pub enum VerifyResult {
    /// Same digest and BLS signature verifies — accept the vote.
    Accept,
    /// Same digest but BLS signature did not verify against our local AttestationData. Either
    /// the sender signed a different `header_hash` (which is in `serialize()` but not in
    /// `digest()`), or the BLS key on file is stale. Treat as a soft reject.
    BadSignature,
    /// Different digest at this height. Local fork or sender fork. The pool will handle this as
    /// a separate fork entry (or as equivocation if the same attestor previously voted otherwise).
    DivergentDigest,
    /// No local AttestationData at this height yet; we can't verify. Drop the vote.
    NoLocal,
    /// The vote's chain_key doesn't match what we're attesting.
    WrongChain,
    /// Sender's BLS public key is not in the active attestor set.
    UnknownAttestor,
}

/// Locally-cached AttestationData for a height (produced by the local production task).
///
/// Stored in the proof cache so verifiers can re-derive the signed message for incoming votes.
#[derive(Clone, Debug)]
pub struct LocalAttestationData {
    pub serialized: Vec<u8>,
    pub digest: Digest,
}

/// Sign a vote with the local BLS key, over the local AttestationData.
pub fn sign_vote(
    bls_key: &bls_signatures::PrivateKey,
    attestor: AttestorId,
    chain_key: ChainKey,
    height: Height,
    local: &LocalAttestationData,
) -> Vote {
    use attestor_primitives::bls::WrapEncode;
    Vote {
        chain_key,
        height,
        digest: local.digest,
        attestor,
        signature_bls: WrapEncode(bls_key.sign(local.serialized.as_slice())),
    }
}

/// Verify an incoming vote against our local AttestationData at the same height.
pub fn verify_vote(
    vote: &Vote,
    our_chain_key: ChainKey,
    local: Option<&LocalAttestationData>,
    pubkey: Option<&bls_signatures::PublicKey>,
) -> VerifyResult {
    if vote.chain_key != our_chain_key {
        return VerifyResult::WrongChain;
    }
    let Some(local) = local else {
        return VerifyResult::NoLocal;
    };
    let Some(pubkey) = pubkey else {
        return VerifyResult::UnknownAttestor;
    };

    if vote.digest != local.digest {
        return VerifyResult::DivergentDigest;
    }
    if pubkey.verify(vote.signature_bls.0, local.serialized.as_slice()) {
        VerifyResult::Accept
    } else {
        VerifyResult::BadSignature
    }
}
