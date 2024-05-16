use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Encode, Decode, TypeInfo, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prover {
    pub nickname: Vec<u8>,
}

#[derive(Debug, Clone, Encode, Decode, TypeInfo, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainPriceConfiguration {
    pub price: u64,
}
