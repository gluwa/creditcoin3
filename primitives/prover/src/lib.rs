#![cfg_attr(not(feature = "std"), no_std)]

use attestor_primitives::ChainId;
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};

pub mod claim;
pub mod claim_query;

#[derive(Debug, Clone, Encode, Decode, TypeInfo, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainPriceConfiguration {
    pub chain_id: ChainId,
    pub price: u64,
}
