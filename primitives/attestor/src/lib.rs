#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::inherent::{InherentIdentifier, IsFatalError};
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::{H256, U256};
use sp_runtime::AccountId32;
use sp_std::vec::Vec;
use starknet_types_core::felt::Felt;

pub mod api;
pub mod bls;

// Chain id to chain name mapping
// Only these are supported for now
pub const CHAIN_ID_TO_CHAIN_NAME: [(u64, &str); 4] = [
    (1, "Ethereum"),
    (31337, "Anvil1"),
    (11_155_111, "Sepolia ethereum"),
    (31338, "Anvil2"),
];

#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
/// Attestor struct
pub struct Attestor<AccountId> {
    pub bls_public_key: Option<BlsPublicKey>,
    pub status: AttestorStatus,
    pub stash: AccountId,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
/// Attestor status
/// Active - Attestor is active and can participate in attestation
/// Idle - Attestor is idle and cannot participate in attestation
pub enum AttestorStatus {
    Active,
    Idle,
}

#[derive(Encode, Decode, Default, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
/// Genesis configuration for attestation pallet
pub struct AttestationChainConfiguration {
    pub chain_key: ChainKey,
    pub attestation_interval: ChainAttestationIntervalType,
    pub attestations_per_checkpoint: u32,
    pub chain_reward: u128,
    pub committee_set_size: u32,
}

/// Identifier for a source chain
pub type ChainId = u64;

/// Mapping key for cc next source chains
pub type ChainKey = u64;

/// Chain attestation interval
pub type ChainAttestationIntervalType = u64;

/// Attestation digest
pub type Digest = H256;

/// BLS public keys as bytes
pub type BlsPublicKey = [u8; 48];

/// BLS signatures as bytes
pub type BlsSignature = [u8; 96];

#[derive(Serialize, Deserialize, Debug, Encode, Decode, PartialEq, Eq)]
pub struct BlsPublicKeyWrapper(#[serde(with = "serde_bytes")] pub BlsPublicKey);

impl BlsPublicKeyWrapper {
    pub fn new(pubkey: BlsPublicKey) -> Self {
        BlsPublicKeyWrapper(pubkey)
    }

    pub fn into_inner(self) -> BlsPublicKey {
        self.0
    }
}

/// Inherent identifier for attestor inherent
pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"attest0r";

#[derive(Encode, Decode, sp_runtime::RuntimeDebug)]
pub enum InherentError {
    NotValid,
    InvalidAttestorFound,
    AttestorNotActive,
    AttestorWithInvalidPublicKey,
    Duplicate(Digest),
}

impl IsFatalError for InherentError {
    fn is_fatal_error(&self) -> bool {
        match self {
            InherentError::NotValid => true,
            InherentError::InvalidAttestorFound => true,
            InherentError::AttestorNotActive => true,
            InherentError::AttestorWithInvalidPublicKey => true,
            InherentError::Duplicate(_) => true,
        }
    }
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestorId(AccountId32);

impl AttestorId {
    pub fn new(id: AccountId32) -> Self {
        Self(id)
    }

    pub fn from_public(public_key: [u8; 32]) -> Self {
        Self(AccountId32::new(public_key))
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.clone().0.into()
    }

    pub fn encode(&self) -> Vec<u8> {
        self.0.encode()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct SignedAttestation<H, AccountId> {
    pub attestation: Attestation<H>,
    pub signature: BlsSignature,
    pub attestors: Vec<AccountId>,
}

impl<H, A> SignedAttestation<H, A>
where
    H: AsRef<[u8]>,
{
    pub fn chain_key(&self) -> ChainKey {
        self.attestation.chain_key
    }

    pub fn header_number(&self) -> u64 {
        self.attestation.header_number
    }

    pub fn digest(&self) -> Digest {
        self.attestation.digest()
    }
}

type ScaleFelt = [u8; 32];

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
pub struct Attestation<H> {
    pub chain_key: ChainKey,
    pub header_number: u64,
    pub header_hash: H,
    pub root: ScaleFelt,
    pub prev_digest: Option<Digest>,
}

impl<H> Attestation<H>
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
        bytes.extend_from_slice(&self.root);

        bytes
    }

    /// Blake2 256 hash from attestation data
    pub fn digest(&self) -> Digest {
        H256::from(&sp_io::hashing::blake2_256(&self.serialize()))
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
    pub block_number: u64,
    pub digest: Digest,
}

pub fn u256_to_felts(x: &U256) -> (Felt, Felt) {
    let mut buf = [0u8; 32];
    x.to_big_endian(&mut buf);
    let lo = Felt::from_bytes_be_slice(&buf[1..32]);
    let hi = Felt::from(buf[0]);

    (lo, hi)
}
