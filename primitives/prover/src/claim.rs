use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};

use attestor_primitives::ChainId;

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, MaxEncodedLen, Hash)]
pub struct Claim<Address> {
    pub chain_id: ChainId,
    pub block_number: u64,
    pub tx_index: u8,
    pub from: Address,
    pub to: Address,
    pub kind: ClaimKind,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    TypeInfo,
    Decode,
    Encode,
    MaxEncodedLen,
    Hash,
    Serialize,
    Deserialize,
)]
pub enum ClaimKind {
    Tx,
    Rx,
}

impl ClaimKind {
    pub fn subdir(&self) -> &str {
        match self {
            Self::Tx => "tx_",
            Self::Rx => "rx_",
        }
    }
}

impl TryFrom<u8> for ClaimKind {
    type Error = ();

    fn try_from(x: u8) -> Result<Self, ()> {
        match x {
            0 => Ok(Self::Tx),
            1 => Ok(Self::Rx),
            _ => Err(()),
        }
    }
}
