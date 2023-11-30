use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::RuntimeDebug;
use sp_std::prelude::*;

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub enum CollectionStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub enum FailureReason {
    UnknownBurn,
    Unauthorized,
    BridgeError,
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub enum BurnId {
    Creditcoin2(u64),
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub struct CollectionInfo {
    pub status: CollectionStatus,
    pub reason: Option<FailureReason>,
}

impl Default for CollectionInfo {
    fn default() -> Self {
        Self {
            status: CollectionStatus::InProgress,
            reason: None,
        }
    }
}
