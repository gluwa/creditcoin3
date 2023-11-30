use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::RuntimeDebug;
use sp_std::prelude::*;

pub type BalanceOf<T> = <<T as crate::Config>::Currency as frame_support::traits::Currency<
    <T as frame_system::Config>::AccountId,
>>::Balance;

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub enum CollectionStatus {
    InProgress,
    Completed,
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub enum FailureReason {
    UnknownBurn,
    BridgeError,
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub enum BurnId {
    Creditcoin2(u64),
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub struct CollectionInfo {
    pub status: CollectionStatus,
}

impl Default for CollectionInfo {
    fn default() -> Self {
        Self {
            status: CollectionStatus::InProgress,
        }
    }
}
