use std::hash::Hash;
use thiserror::Error;

use mmr::{proof::Proof, Mmr};
pub use starknet_crypto::FieldElement;

use super::pedersen::StarknetPedersenHash;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
/// `TElement` of a merkletree
pub struct TElement(pub Vec<u8>);

impl AsRef<[u8]> for TElement {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub type StarknetPedersenMmr = Mmr<StarknetPedersenHash>;
pub type StarknetPedersenMerkleProof = Proof<StarknetPedersenHash>;

/// Result type
pub type Result = std::result::Result<StarknetPedersenMmr, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error creating tree")]
    ErrorCreatingTree,
    #[error("Other error")]
    OtherError(#[from] anyhow::Error),
}

/// Create a merkletree given input of byte slices
/// We need atleast a vector of length 2 otherwise we cannot construct a merkle tree
pub fn create(rlps: &[Vec<u8>]) -> Result {
    if rlps.is_empty() {
        return Err(Error::ErrorCreatingTree);
    }

    let tree = StarknetPedersenMmr::from(&rlps[..]);

    Ok(tree)
}
