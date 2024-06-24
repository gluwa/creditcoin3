#![cfg_attr(not(feature = "std"), no_std)]

pub mod block_item_traits;

#[cfg(feature = "std")]
pub mod json_serializable;
pub mod pedersen_hash;
pub mod utils;

use crate::pedersen_hash::StarknetPedersenHash;
use mmr::{proof::Proof, Mmr};

pub type Felt = starknet_crypto::FieldElement;

pub type StarknetPedersenMmr = Mmr<StarknetPedersenHash>;
pub type StarknetPedersenMerkleProof = Proof<StarknetPedersenHash>;

// #[cfg(feature = "std")]
// pub fn print_with_timestamp(s: colored::ColoredString) {
//     println!(
//         "[{}] {}",
//         chrono::Local::now().time().format("%H:%M:%S%.3f"),
//         s
//     );
// }
