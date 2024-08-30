#![cfg_attr(not(feature = "std"), no_std)]

pub mod block_item_traits;

#[cfg(feature = "std")]
pub mod json_serializable;
pub mod pedersen_hash;
pub mod utils;

use crate::pedersen_hash::StarknetPedersenHash;
use mmr::{proof::Proof, BaseTree};

pub type Felt = starknet_crypto::Felt;

//pub type StarknetPedersenMmr = Mmr<StarknetPedersenHash>;
pub type StarknetPedersenMerkleTree = BaseTree<StarknetPedersenHash>;
pub type StarknetPedersenMerkleProof = Proof<StarknetPedersenHash>;
