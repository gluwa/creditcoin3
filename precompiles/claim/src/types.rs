use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use precompile_utils::solidity::Codec;

use attestor_primitives::ChainId;

#[derive(Clone, Debug, PartialEq, Eq, Decode, Encode, MaxEncodedLen, Hash, Codec)]
pub struct EvmClaim<Address> {
    pub chain_id: ChainId,
    pub block_number: u64,
    pub tx_index: u8,
    pub from: Address,
    pub to: Address,
    pub is_tx: bool,
    pub is_rx: bool,
}
