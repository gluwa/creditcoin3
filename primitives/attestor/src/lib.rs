#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::inherent::{InherentIdentifier, IsFatalError};
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_std::vec::Vec;

pub type Felt = [u8; 32];

pub type ChainId = u8;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Encode, Decode, MaxEncodedLen, TypeInfo)]
pub struct SignedAttestation<H> {
    pub attestation_data: AttestationData<H>,
    pub digest: Digest,
    pub signature: BlsSignature,
}

impl<H> SignedAttestation<H> {
    pub fn chain_id(&self) -> ChainId {
        self.attestation_data.chain_id
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
)]
pub struct AttestationData<H> {
    pub chain_id: ChainId,
    pub header_number: u64,
    pub header_hash: H,
    pub tx_root: Felt,
    pub rx_root: Felt,
}

pub type Digest = H256;

impl<H> AttestationData<H>
where
    H: AsRef<[u8]>,
{
    #[must_use]
    pub fn serialize(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Serialize chain_id as little-endian bytes
        bytes.extend_from_slice(self.chain_id.to_le_bytes().as_ref());

        // Serialize header_number as little-endian bytes
        bytes.extend_from_slice(self.header_number.to_le_bytes().as_ref());

        // Serialize header_hash
        bytes.extend_from_slice(self.header_hash.as_ref());

        // Serialize tx_root
        bytes.extend_from_slice(&self.tx_root);

        // Serialize rx_root
        bytes.extend_from_slice(&self.rx_root);

        bytes
    }

    /// Blake2 256 hash from attestation data
    pub fn digest(&self) -> Digest {
        H256::from(&sp_io::hashing::blake2_256(&self.serialize()))
    }
}
