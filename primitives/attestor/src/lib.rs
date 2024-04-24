#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::inherent::{InherentIdentifier, IsFatalError};
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;
use sp_std::vec::Vec;

pub type Felt = [u8; 32];

pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"attest0r";

#[derive(Encode, Decode, sp_runtime::RuntimeDebug)]
// #[cfg_attr(feature = "std", derive(Decode))]
pub enum InherentError {
    NotValid,
    Duplicate,
}

impl IsFatalError for InherentError {
    fn is_fatal_error(&self) -> bool {
        match self {
            InherentError::NotValid => true,
            InherentError::Duplicate => true,
        }
    }
}

pub type BlsSignature = [u8; 42];

#[derive(Debug, Clone, Decode, Encode)]
pub struct AttestationInherentData {
    pub chain_id: u8,
    pub block_number: u64,
    pub signature: BlsSignature,
    pub tx_root: H256,
    pub rx_root: H256,
    pub digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AttestationData {
    pub chain_id: u8,
    pub header_number: u64,
    pub header_hash: H256,
    pub tx_root: Felt,
    pub rx_root: Felt,
}

pub type Digest = H256;

impl AttestationData {
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize header_number as little-endian bytes
        bytes.extend_from_slice(self.header_number.to_be_bytes().as_ref());

        // Serialize header_hash as little-endian bytes
        bytes.extend_from_slice(self.header_hash.0.as_ref());

        // Serialize tx_root as little-endian bytes
        bytes.extend_from_slice(&self.tx_root);

        // Serialize rx_root as little-endian bytes
        bytes.extend_from_slice(&self.rx_root);

        bytes
    }

    /// Blake2 256 hash from attestation data
    pub fn digest(&self) -> Digest {
        H256::from(&sp_io::hashing::blake2_256(&self.serialize()))
    }
}
