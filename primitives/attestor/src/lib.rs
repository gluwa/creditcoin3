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

/// Function to calculate the threshold for a committee set size to reach majority vote.
///
/// `sample_size` is the *effective* committee — see [`effective_sample_size`]. Callers that hold
/// a raw `TargetSampleSize` must not pass it here directly; use [`calculate_quorum`], which
/// applies the active-set cap first.
///
/// The multiply saturates. `TargetSampleSize` is only bounded by `> 0` at the setter, so a
/// governance value above `u32::MAX / 2` would otherwise wrap in a release build (the workspace
/// release profile does not enable `overflow-checks`) and collapse the threshold to a handful of
/// signers. Saturating keeps an absurd configuration unreachable-high instead of dangerously low.
pub fn calculate_threshold(sample_size: u32) -> u32 {
    sample_size.saturating_mul(2) / 3 + 1
}

/// The effective committee a quorum is measured against: `min(active_attestors, target_sample_size)`.
///
/// `TargetSampleSize` is a *cap*, not a fixed committee size. When fewer attestors are active than
/// the cap, the whole active set is the committee; the cap only binds once the set grows past it.
///
/// This fixes a hard liveness bug in the previous `threshold = f(target_sample_size)` model: a
/// target above the active-attestor count made the threshold unreachable, so `commit_attestation`
/// failed `MajorityNotReached` forever and attestation for the chain stopped permanently.
///
/// Note what the cap does *not* do. Nothing selects a committee — `validate_attestation` accepts
/// any subset of `ActiveAttestors` that clears the threshold, and the per-epoch entropy that would
/// drive sortition is still unused (`do_start_election`'s `_randomness`, RFC-0174). So while the
/// cap binds (active > target) any self-selected `2/3+1` of the *cap* is a valid quorum, and two
/// disjoint such groups can exist. Quorum intersection only holds while the cap does not bind, so
/// operators who need it must keep `TargetSampleSize` above the active-attestor count.
pub fn effective_sample_size(active_attestors: u32, target_sample_size: u32) -> u32 {
    active_attestors.min(target_sample_size)
}

/// Quorum threshold for a chain: `2/3 + 1` of [`effective_sample_size`].
///
/// This is the single definition shared by the runtime (`validate_attestation`) and the attestor
/// node. Both sides must agree: an attestor computing a threshold the runtime does not enforce
/// either burns fees on `MajorityNotReached` (too low) or never submits at all (too high).
pub fn calculate_quorum(active_attestors: u32, target_sample_size: u32) -> u32 {
    calculate_threshold(effective_sample_size(active_attestors, target_sample_size))
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
        let target_sample_size = 3;
        let threshold = calculate_threshold(target_sample_size);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_4() {
        let target_sample_size = 4;
        let threshold = calculate_threshold(target_sample_size);
        assert_eq!(threshold, 3);
    }

    #[test]
    fn test_calculate_threshold_5() {
        let target_sample_size = 5;
        let threshold = calculate_threshold(target_sample_size);
        assert_eq!(threshold, 4);
    }

    #[test]
    fn test_calculate_threshold_10() {
        let target_sample_size = 10;
        let threshold = calculate_threshold(target_sample_size);
        assert_eq!(threshold, 7);
    }

    #[test]
    fn effective_sample_size_uses_active_set_when_below_target() {
        assert_eq!(effective_sample_size(10, 20), 10);
    }

    #[test]
    fn effective_sample_size_uses_target_when_active_set_is_larger() {
        assert_eq!(effective_sample_size(100, 20), 20);
    }

    #[test]
    fn effective_sample_size_is_the_common_value_when_equal() {
        assert_eq!(effective_sample_size(9, 9), 9);
    }

    /// Regression for the liveness bug this change fixes: a target above the active-attestor
    /// count used to yield a threshold no quorum could ever reach, halting the chain. The local
    /// testnet spec ships `target_sample_size: 9`, which needed 7 active attestors.
    #[test]
    fn quorum_is_reachable_when_target_exceeds_active_set() {
        let active = 3;
        assert_eq!(calculate_threshold(9), 7, "old model needed 7 of 3");
        let quorum = calculate_quorum(active, 9);
        assert_eq!(quorum, 3);
        assert!(
            quorum <= active,
            "quorum must be reachable by the active set"
        );
    }

    /// Quorum must never exceed the active set for *any* target, or attestation cannot progress.
    #[test]
    fn quorum_never_exceeds_the_active_set() {
        for active in 1u32..64 {
            for target in [1u32, 3, 9, 20, 100, 20_000, u32::MAX] {
                let quorum = calculate_quorum(active, target);
                assert!(
                    quorum <= active,
                    "quorum {quorum} > active {active} (target {target})"
                );
                assert!(quorum >= 1, "quorum must be positive");
            }
        }
    }

    /// While the cap binds, quorum intersection is lost — this is the documented consequence of
    /// capping without sortition, asserted so the tradeoff cannot regress silently.
    #[test]
    fn quorum_intersects_only_while_the_cap_does_not_bind() {
        // Cap does not bind: quorum is a strict majority of the active set, so any two quorums
        // must share a member.
        let active = 100;
        let uncapped = calculate_quorum(active, u32::MAX);
        assert!(2 * uncapped > active, "uncapped quorum must intersect");

        // Cap binds: two disjoint quorums fit inside the active set.
        let capped = calculate_quorum(active, 20);
        assert_eq!(capped, 14);
        assert!(2 * capped <= active, "capped quorum does not intersect");
    }

    /// A governance target above `u32::MAX / 2` must not wrap the `* 2` and collapse the
    /// threshold. The setter only rejects zero, so this input is reachable.
    #[test]
    fn calculate_threshold_saturates_instead_of_wrapping() {
        assert_eq!(calculate_threshold(u32::MAX), u32::MAX / 3 + 1);
        assert_eq!(calculate_quorum(10, u32::MAX), 7);
    }
}
