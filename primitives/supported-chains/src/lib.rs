#![cfg_attr(not(feature = "std"), no_std)]
pub mod api;
pub mod provider;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, TypeInfo)]
pub struct SupportedChain {
    pub chain_id: u64,
    pub chain_name: Vec<u8>,
}
