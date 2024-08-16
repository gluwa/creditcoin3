#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::inherent::{InherentIdentifier, IsFatalError};
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::{H256, U256};
use sp_std::vec::Vec;
use starknet_types_core::felt::Felt;

pub mod api;
pub mod bls;

pub type ChainId = u64;

pub type ChainKey = u64;

/// BLS public keys as bytes
pub type BlsPublicKey = [u8; 48];

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

pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"attest0r";

#[derive(Encode, Decode, sp_runtime::RuntimeDebug)]
pub enum InherentError {
    NotValid,
    Duplicate(Digest),
}

impl IsFatalError for InherentError {
    fn is_fatal_error(&self) -> bool {
        match self {
            InherentError::NotValid => true,
            InherentError::Duplicate(_) => true,
        }
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
    pub fn chain_id(&self) -> ChainId {
        self.attestation.chain_id
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
    pub chain_id: ChainId,
    pub header_number: u64,
    pub header_hash: H,
    pub tx_root: ScaleFelt,
    pub rx_root: ScaleFelt,
    pub prev_digest: Option<Digest>,
}

pub type Digest = H256;

impl<H> Attestation<H>
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

    // This seems to break the attestations, they are not going through smoothly if I use pedersen hash instead of blake
    // Need to investigate more
    // pub fn digest(&self) -> Digest {
    //     let (lo, hi) = u256_to_felts(&self.header_number.into());
    //     let d1 = starknet_crypto::pedersen_hash(&lo, &hi);
    //     let d2 = starknet_crypto::pedersen_hash(&d1, &Felt::from_bytes_be_slice(&self.tx_root));
    //     let d3 = starknet_crypto::pedersen_hash(&d2, &Felt::from_bytes_be_slice(&self.rx_root));
    //
    //     starknet_crypto::pedersen_hash(
    //         &d3,
    //         &Felt::from_bytes_be_slice(
    //             self.prev_digest
    //                 .unwrap_or_else(|| [0u8; 32].into())
    //                 .as_bytes(),
    //         ),
    //     )
    //     .to_bytes_be()
    //     .into()
    // }
}

pub fn u256_to_felts(x: &U256) -> (Felt, Felt) {
    let mut buf = [0u8; 32];
    x.to_big_endian(&mut buf);
    let lo = Felt::from_bytes_be_slice(&buf[1..32]);
    let hi = Felt::from(buf[0]);

    (lo, hi)
}
