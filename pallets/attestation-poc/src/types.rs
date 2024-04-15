use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::hash::H256;
use sp_core::RuntimeDebug;

pub type BlockNumber = u64;
pub type Digest = H256;
pub type ChainId = u8;

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct Attestation<B>
where
    B: Clone + Send + Sync,
{
    pub block_number: BlockNumber,
    pub bls: B,
    // Tx Root is a starknet crypto FieldElement, which is a 32 byte hash
    pub tx_root: H256,
    // Rx Root is a starknet crypto FieldElement, which is a 32 byte hash
    pub rx_root: H256,
    pub digest: Digest,
    pub prev_digest: Digest,
}

impl<B> Attestation<B>
where
    B: Clone + Send + Sync,
{
    pub fn new(input: AttestationInput<B>, prev_digest: Digest) -> Self {
        Self {
            block_number: input.block_number,
            bls: input.bls,
            tx_root: input.tx_root,
            rx_root: input.rx_root,
            digest: input.digest,
            prev_digest,
        }
    }
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct AttestationInput<B>
where
    B: Clone + Send + Sync,
{
    pub block_number: BlockNumber,
    pub bls: B,
    // Tx Root is a starknet crypto FieldElement, which is a 32 byte hash
    pub tx_root: H256,
    // Rx Root is a starknet crypto FieldElement, which is a 32 byte hash
    pub rx_root: H256,
    pub digest: H256,
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct InherentType<B>
where
    B: Clone + Send + Sync,
{
    pub block_number: BlockNumber,
    pub chain_id: ChainId,
    pub attestation: AttestationInput<B>,
}
