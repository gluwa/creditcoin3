use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, MaxEncodedLen, Hash)]
pub struct Claim<Address> {
    pub chain_id: u8,
    pub block_number: u64,
    pub tx_index: u8,
    pub from: Address,
    pub to: Address,
    pub kind: ClaimKind,
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, MaxEncodedLen, Hash)]
pub enum ClaimKind {
    Tx,
    Rx,
}
