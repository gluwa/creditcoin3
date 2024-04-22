use frame_support::inherent::InherentIdentifier;
use parity_scale_codec::{Decode, Encode};
use sp_core::H256;

pub type Felt = [u8; 32];

pub const INHERENT_IDENTIFIER: InherentIdentifier = *b"attest0r";

#[derive(Encode, Decode, sp_runtime::RuntimeDebug)]
// #[cfg_attr(feature = "std", derive(Decode))]
pub enum InherentError {
    NotValid,
    Duplicate,
}

#[derive(Debug, Clone)]
pub struct AttestationData {
    pub header_number: u64,
    pub header_hash: H256,
    pub tx_root: Felt,
    pub rx_root: Felt,
}

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
}
