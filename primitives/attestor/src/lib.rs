#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

use parity_scale_codec::{Decode, DecodeWithMemTracking, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_runtime::AccountId32;
use sp_std::vec::Vec;

pub mod api;
pub mod block;
pub mod bls;
// Re-export block types for convenience
pub use block::{Block, ContinuityBlock, ContinuityProof};

use crate::bls::{Bls, CryptoScheme};

#[derive(Encode, Decode, DecodeWithMemTracking, Clone, PartialEq, Eq, Debug, TypeInfo)]
/// Attestor struct
pub struct Attestor<AccountId> {
    pub bls_public_key: Option<BlsPublicKey>,
    pub status: AttestorStatus,
    pub stash: AccountId,
}

#[derive(Encode, Decode, DecodeWithMemTracking, Clone, PartialEq, Eq, Debug, TypeInfo)]
/// Attestor status
/// Active - Attestor is active and can participate in attestation
/// Idle - Attestor is idle and cannot participate in attestation
/// Waiting - Attestor is waiting for the next attestation round
/// Leaving - Voluntary chill scheduled; remains in the current epoch committee until the next election
pub enum AttestorStatus {
    Active = 0,
    Idle = 1,
    Waiting = 2,
    Leaving = 3,
}

impl AttestorStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(
    Encode,
    Decode,
    DecodeWithMemTracking,
    Default,
    Clone,
    PartialEq,
    Eq,
    Deserialize,
    serde::Serialize,
)]
/// Genesis configuration for attestation pallet
pub struct AttestationChainConfiguration {
    pub chain_key: ChainKey,
    pub attestation_interval: ChainAttestationIntervalType,
    pub attestations_per_checkpoint: u32,
    pub target_sample_size: u32,
    pub checkpoints: Vec<AttestationCheckpoint>,
}

#[derive(
    Serialize,
    Deserialize,
    Debug,
    Copy,
    Clone,
    Encode,
    Decode,
    DecodeWithMemTracking,
    TypeInfo,
    PartialEq,
    Eq,
)]
/// Encoding version to use when processing blocks from source chains
pub enum ChainEncodingVersion {
    V1 = 1,
}

#[cfg(feature = "std")]
impl From<ChainEncodingVersion> for usc_abi_encoding::common::EncodingVersion {
    fn from(version: ChainEncodingVersion) -> Self {
        match version {
            ChainEncodingVersion::V1 => usc_abi_encoding::common::EncodingVersion::V1,
        }
    }
}

/// Identifier for a source chain
pub type ChainId = u64;

/// Mapping key for cc next source chains
pub type ChainKey = u64;

/// Chain attestation interval
pub type ChainAttestationIntervalType = u64;

/// Attestation digest
pub type Digest = H256;

/// Block height
pub type Height = u64;

/// BLS public keys as bytes
pub type BlsPublicKey = [u8; 48];

/// BLS signatures as bytes
pub type BlsSignature = [u8; 96];

/// Domain-separation tag for the BLS proof-of-possession message.
///
/// Versioned so a future change to the message layout is a different domain rather than an
/// ambiguous re-encoding: a proof built for `v1` can never verify under a later scheme.
pub const POP_DOMAIN_V1: &[u8] = b"CC3:ATTESTOR:POP:v1";

/// The message a BLS proof of possession must be signed over.
///
/// The proof exists to show that whoever submits `bls_public_key` actually holds the
/// corresponding private key. Signing the public key *alone* proves possession of the key but
/// says nothing about **who** is claiming it, and the proof travels in the clear as an argument
/// of the public `attest` extrinsic — so anyone who has seen a victim's registration can replay
/// their proof under a different controller and take ownership of `BlsKeyOwner` for that key.
/// The victim is then permanently locked out with `BlsKeyAlreadyRegistered`, and the squatter
/// holds a committee slot it cannot service: it counts toward the quorum denominator while never
/// being able to produce a signature, which is a liveness problem, not just a nuisance.
///
/// Binding `chain_key` and `attestor_id` into the message makes a proof usable only by the
/// account that claims it, on the chain it was built for. `attest` derives `attestor_id` from
/// `ensure_signed(origin)` and is the only caller of `start_attesting`, so a third party cannot
/// submit a bound proof at all — they would have to sign the extrinsic as the victim. That is
/// also why the message carries no nonce or expiry: the only party who can replay a bound proof
/// is its owner, re-asserting their own key, which the pallet already treats as an idempotent
/// no-op.
///
/// `attestor_id` is taken as raw bytes so the runtime (generic over `T::AccountId`) and the
/// off-chain attestor (concrete `AccountId32`) can both reach this one definition. Callers pass
/// the SCALE encoding of the account; for `AccountId32` that is its 32 raw bytes.
///
/// Changing this layout is a coordinated upgrade: an attestor binary signing the old message
/// cannot register against a runtime verifying the new one. Already-active attestors are
/// unaffected until they chill, since `attest` is only called from `Idle`.
///
/// **Both sides must build the message here.** A second implementation that agrees today is a
/// second implementation that can drift tomorrow, and the failure mode is every attestor being
/// unable to register.
#[must_use]
pub fn proof_of_possession_message(
    chain_key: ChainKey,
    attestor_id: &[u8],
    bls_public_key: &BlsPublicKey,
) -> Vec<u8> {
    let mut message =
        Vec::with_capacity(POP_DOMAIN_V1.len() + 8 + attestor_id.len() + bls_public_key.len());
    message.extend_from_slice(POP_DOMAIN_V1);
    message.extend_from_slice(&chain_key.to_le_bytes());
    message.extend_from_slice(attestor_id);
    message.extend_from_slice(bls_public_key);
    message
}

#[derive(Serialize, Deserialize, Debug, Encode, Decode, DecodeWithMemTracking, PartialEq, Eq)]
pub struct BlsPublicKeyWrapper(#[serde(with = "serde_bytes")] pub BlsPublicKey);

impl BlsPublicKeyWrapper {
    pub fn new(pubkey: BlsPublicKey) -> Self {
        BlsPublicKeyWrapper(pubkey)
    }

    pub fn into_inner(self) -> BlsPublicKey {
        self.0
    }
}

#[derive(Encode, Decode, Debug, Clone, PartialOrd, Ord, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "std", derive(Hash))]
pub struct AttestorId(AccountId32);

// AccountId32 is [u8; 32] - fixed-size, no heap allocation.
impl DecodeWithMemTracking for AttestorId {}

impl AttestorId {
    pub const fn new(id: AccountId32) -> Self {
        Self(id)
    }

    pub const fn from_public(public_key: [u8; 32]) -> Self {
        Self(AccountId32::new(public_key))
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.clone().0.into()
    }

    pub fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }

    pub fn account_id(&self) -> &AccountId32 {
        &self.0
    }
}

impl core::fmt::Display for AttestorId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        use sp_core::crypto::Ss58Codec;
        write!(f, "{}", self.0.to_ss58check())
    }
}

impl From<AttestorId> for [u8; 32] {
    fn from(attestor_id: AttestorId) -> [u8; 32] {
        attestor_id.0.into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, TypeInfo)]
pub struct SignedAttestation<H, AccountId> {
    pub attestation: AttestationData<H>,
    pub signature: BlsSignature,
    pub attestors: Vec<AccountId>,
    pub continuity_proof: ContinuityProof,
}

impl<H, A> SignedAttestation<H, A>
where
    H: AsRef<[u8]>,
{
    pub fn chain_key(&self) -> ChainKey {
        self.attestation.chain_key
    }

    pub fn header_number(&self) -> Height {
        self.attestation.header_number
    }

    pub fn digest(&self) -> Digest {
        self.attestation.digest()
    }

    pub fn prev_digest(&self) -> Option<Digest> {
        self.attestation.prev_digest()
    }

    pub fn round(&self) -> Round {
        self.attestation.round()
    }
}

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attestation<H, AccountId> {
    pub attestation_data: AttestationData<H>,
    pub attestor: AccountId,
    pub signature: sp_core::sr25519::Signature,
    pub signature_bls: <Bls as CryptoScheme>::Signature,
    pub continuity_proof: ContinuityProof,
}

// sr25519::Signature (CryptoBytes) and BLS sig (WrapEncode) are fixed-size crypto types.
impl<H: DecodeWithMemTracking, AccountId: DecodeWithMemTracking> DecodeWithMemTracking
    for Attestation<H, AccountId>
{
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

    pub fn header_number(&self) -> Height {
        self.attestation_data.header_number
    }

    pub fn attestor_id(&self) -> AttestorId {
        AttestorId::from_public(self.attestor.clone().into())
    }
}
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    MaxEncodedLen,
    TypeInfo,
    Default,
)]
pub struct AttestationData<H> {
    pub chain_key: ChainKey,
    pub header_number: Height,
    pub header_hash: H,
    pub root: H256,
    pub prev_digest: Option<Digest>,
}

// H256/Digest are fixed-size with no heap allocation.
impl<H: DecodeWithMemTracking> DecodeWithMemTracking for AttestationData<H> {}

/// Attestation round
/// Is the chain key and the header number
pub type Round = (ChainKey, Height);

impl AttestationData<Digest> {
    pub fn new(
        chain_key: ChainKey,
        header_number: Height,
        header_hash: Digest,
        root: H256,
        prev_digest: Option<Digest>,
    ) -> Self {
        AttestationData {
            chain_key,
            header_number,
            header_hash,
            root,
            prev_digest,
        }
    }
}

impl<H> AttestationData<H>
where
    H: AsRef<[u8]>,
{
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize chain_key as little-endian bytes
        bytes.extend_from_slice(self.chain_key.to_le_bytes().as_ref());

        // Serialize header_number as little-endian bytes
        bytes.extend_from_slice(self.header_number.to_le_bytes().as_ref());

        // Serialize header_hash
        bytes.extend_from_slice(self.header_hash.as_ref());

        // Serialize tx_root
        bytes.extend_from_slice(self.root.as_bytes());

        // Serialize prev_digest if it exists
        if let Some(prev_digest) = &self.prev_digest {
            bytes.extend_from_slice(prev_digest.as_ref());
        }

        bytes
    }

    /// Digest for the attestation is the keccak256 hash of the header number, root,
    /// and the previous digest if it exists
    pub fn digest(&self) -> Digest {
        compute_digest_for(self.header_number, &self.root, self.prev_digest.as_ref())
    }

    pub fn prev_digest(&self) -> Option<Digest> {
        self.prev_digest
    }

    pub fn round(&self) -> Round {
        (self.chain_key, self.header_number)
    }

    pub fn chain_key(&self) -> ChainKey {
        self.chain_key
    }

    pub fn header_number(&self) -> Height {
        self.header_number
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    MaxEncodedLen,
    TypeInfo,
    Default,
)]
pub struct AttestationCheckpoint {
    pub block_number: Height,
    pub digest: Digest,
}

// Digest = H256 which is fixed-size with no heap allocation.
impl DecodeWithMemTracking for AttestationCheckpoint {}

impl AttestationCheckpoint {
    pub fn new(block_number: Height, digest: Digest) -> Self {
        Self {
            block_number,
            digest,
        }
    }

    pub fn block_number(&self) -> Height {
        self.block_number
    }

    pub fn digest(&self) -> Digest {
        self.digest
    }
}

/// `2/3 + 1` of `active_attestors` — the bare arithmetic behind [`calculate_quorum`].
///
/// Prefer [`calculate_quorum`]; this is split out only so the formula can be asserted directly.
/// Whatever is passed here *is* the set the threshold is measured against, so passing anything
/// smaller than the live active-attestor count hands a minority of that set a passing quorum.
///
/// The multiply saturates. In-tree callers pass a count bounded by `MaxAttestationNodes`, far
/// below the wrapping point, but this is `pub`: an input above `u32::MAX / 2` would otherwise wrap
/// in a release build (the workspace release profile does not enable `overflow-checks`) and
/// collapse the threshold to a handful of signers. Saturating keeps an absurd input
/// unreachable-high instead of dangerously low.
pub fn calculate_threshold(active_attestors: u32) -> u32 {
    active_attestors.saturating_mul(2) / 3 + 1
}

/// Quorum threshold for a chain: `2/3 + 1` of the live active-attestor count.
///
/// The committee **is** `ActiveAttestors`: `elect_attestors_for_chain` selects every eligible
/// attestor and nothing samples a subset. Deriving the threshold from anything smaller would let
/// a self-selected minority of that set clear it — with 30 active and a cap of 9 the threshold
/// was 7, so two disjoint groups of seven could each produce a conflicting quorum for the same
/// height. Measuring against the set the signers are actually drawn from is what makes quorum
/// intersection hold: an adversary below `1/3` of the active set can never reach `2/3+1` of it.
///
/// `TargetSampleSize` deliberately plays no part here. It is reserved for the committee sortition
/// in RFC-0174, which is out of scope at the current `MaxAttestationNodes` ceiling; see its
/// storage docs in `pallet-attestation`.
///
/// This is the single definition shared by the runtime (`validate_attestation`) and the attestor
/// node. Both sides must agree: an attestor computing a threshold the runtime does not enforce
/// either burns fees on `MajorityNotReached` (too low) or never submits at all (too high).
///
/// Reachable at every set size, including the small ones. An earlier model derived the threshold
/// from `TargetSampleSize` alone, so a target above the active-attestor count was unsatisfiable
/// and attestation for that chain halted permanently. Deriving it from the live count cannot
/// reproduce that: the threshold is always at most the number of attestors able to sign.
pub fn calculate_quorum(active_attestors: u32) -> u32 {
    calculate_threshold(active_attestors)
}

/// Computes the digest for a block given its number, root, and optional previous digest.
///
/// Build input bytes: header_number || root || prev_digest (if exists)
#[must_use]
#[inline]
pub fn compute_digest_for(block_number: u64, root: &H256, prev_digest: Option<&H256>) -> H256 {
    use sha3::{Digest, Keccak256};

    let result: [u8; 32] = Keccak256::new()
        .chain_update(block_number.to_be_bytes())
        .chain_update(root.as_bytes())
        .chain_update(prev_digest.map(H256::as_bytes).unwrap_or_default())
        .finalize()
        .into();

    H256(result)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_calculate_threshold_3() {
        let active_attestors = 3;
        let threshold = calculate_threshold(active_attestors);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_4() {
        let active_attestors = 4;
        let threshold = calculate_threshold(active_attestors);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_5() {
        let active_attestors = 5;
        let threshold = calculate_threshold(active_attestors);
        assert_eq!(threshold, 4);
    }

    #[test]
    fn test_calculate_threshold_10() {
        let active_attestors = 10;
        let threshold = calculate_threshold(active_attestors);
        assert_eq!(threshold, 7);
    }

    /// The property the whole design rests on: quorum is a strict majority of the set the signers
    /// are drawn from, so any two quorums must share at least one member. Previously, capping the
    /// threshold at a smaller `TargetSampleSize` broke this once the cap bound — two disjoint
    /// groups could each clear it and attest conflicting roots at the same height.
    #[test]
    fn two_quorums_always_intersect() {
        for active in 1u32..256 {
            let quorum = calculate_quorum(active);
            assert!(
                2 * quorum > active,
                "two disjoint quorums fit in {active} active (quorum {quorum})"
            );
        }
    }

    /// An adversary below one third of the active set can never reach the threshold. This is what
    /// deriving the threshold from the live count buys, and what any reintroduced sampling would
    /// have to re-establish on the sampled committee instead.
    #[test]
    fn a_minority_under_one_third_can_never_reach_quorum() {
        for active in 3u32..256 {
            let adversary = (active - 1) / 3; // strictly under 1/3
            assert!(
                adversary < calculate_quorum(active),
                "{adversary} of {active} reached quorum {}",
                calculate_quorum(active)
            );
        }
    }

    /// Regression for the liveness bug `4603d0fa8` fixed: deriving the threshold from a target
    /// above the active count made it unreachable and halted the chain permanently. Deriving it
    /// from the live count cannot reproduce that at any set size.
    #[test]
    fn quorum_never_exceeds_the_active_set() {
        assert_eq!(calculate_threshold(9), 7, "old model needed 7 of 3");
        for active in 1u32..256 {
            let quorum = calculate_quorum(active);
            assert!(quorum <= active, "quorum {quorum} > active {active}");
            assert!(quorum >= 1, "quorum must be positive");
        }
    }

    /// `TargetSampleSize` is no longer an input, so no governance value can collapse the
    /// threshold. The saturating multiply still guards an absurd active count.
    #[test]
    fn calculate_threshold_saturates_instead_of_wrapping() {
        assert_eq!(calculate_threshold(u32::MAX), u32::MAX / 3 + 1);
        assert_eq!(calculate_quorum(u32::MAX), u32::MAX / 3 + 1);
    }
}

#[cfg(test)]
mod pop_message_tests {
    use super::*;

    /// Fixed vector, duplicated byte-for-byte in `cli/src/lib/attestor/proof-of-possession.ts`.
    ///
    /// The TypeScript integration tests have to build this message independently, so the two
    /// implementations can silently diverge — and the symptom would be every attestor failing to
    /// register. Pinning the same vector on both sides turns that into a test failure instead.
    #[test]
    fn message_layout_is_pinned() {
        let chain_key: ChainKey = 2;
        let attestor_id = [0x11u8; 32];
        let bls_public_key: BlsPublicKey = [0x22u8; 48];

        let message = proof_of_possession_message(chain_key, &attestor_id, &bls_public_key);

        let mut expected = Vec::new();
        expected.extend_from_slice(b"CC3:ATTESTOR:POP:v1");
        expected.extend_from_slice(&[2, 0, 0, 0, 0, 0, 0, 0]);
        expected.extend_from_slice(&attestor_id);
        expected.extend_from_slice(&bls_public_key);

        assert_eq!(message, expected);
        assert_eq!(message.len(), 19 + 8 + 32 + 48);
    }
}
