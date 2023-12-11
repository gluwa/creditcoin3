use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use sp_core::RuntimeDebug;
use sp_std::prelude::*;

pub type BalanceFor<T> = <<T as crate::Config>::Currency as frame_support::traits::Currency<
    <T as frame_system::Config>::AccountId,
>>::Balance;

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub struct Cc2BurnId(pub u64);

#[derive(Clone, Encode, Decode, Eq, PartialEq, TypeInfo, MaxEncodedLen, RuntimeDebug)]
pub struct CollectionInfo<AccountId, Balance, BlockNumber> {
    pub amount: Balance,
    pub collector: AccountId,
    pub block_number: BlockNumber,
}
