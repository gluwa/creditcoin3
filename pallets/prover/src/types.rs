use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_std::vec::Vec;
use attestor_primitives::ChainId;

#[derive(Debug, Clone, Encode, Decode, TypeInfo, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prover {
    pub nickname: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash)]
pub struct Claim {
    pub chain_id: ChainId,
    pub id: ClaimId,
    pub felt_ranges: Vec<FeltRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash)]
pub struct ClaimId {
    pub kind: ClaimKind,
    pub block_item_id: BlockItemIdentifier,
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash)]
pub struct BlockItemIdentifier {
    pub block_number: u64,
    pub index: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash)]
pub struct FeltRange {
    pub start: u32,
    pub end: u32,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Serialize, Deserialize,
)]
pub enum ClaimKind {
    Tx = 1,
    Rx = 2,
}
