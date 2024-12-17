use parity_scale_codec::{Decode, Encode};
use sp_runtime::traits::Block as BlockT;

use super::Attestation;
use crate::HashFor;

#[derive(Debug, PartialEq)]
pub enum Action<H> {
    Keep(H),
    Discard,
}

#[derive(Encode, Decode, Debug, Clone)]
pub enum Message<B: BlockT, AccountId> {
    Attestation(Attestation<HashFor<B>, AccountId>),
}
