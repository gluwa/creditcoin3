use attestor_primitives::ChainId;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;

#[derive(Debug, Clone, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct Prover {
    pub nickname: Vec<u8>,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo, PartialEq, Eq)]
pub struct ChainPriceConfiguration {
    pub chain_id: ChainId,
    pub price: u64,
}
