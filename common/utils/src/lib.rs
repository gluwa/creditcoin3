pub mod pedersen_hash;
pub mod utils;
pub mod json_serializable; 
pub mod block_item_traits;

use crate::pedersen_hash::StarknetPedersenHash;
use mmr::{Mmr, proof::Proof};

pub type Felt = starknet_crypto::FieldElement;

pub type StarknetPedersenMmr = Mmr<StarknetPedersenHash>;
pub type StarknetPedersenMerkleProof = Proof<StarknetPedersenHash>;

pub fn print_with_timestamp(s: colored::ColoredString) {
    println!(
        "[{}] {}",
        chrono::Local::now().time().format("%H:%M:%S%.3f"),
        s
    );
}

