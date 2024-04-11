use attestor_primitives::Felt;
use merkletree::{merkle::Element, proof::Proof, store::VecStore};
use std::hash::Hash;
use thiserror::Error;
use tracing::debug;

pub use starknet_crypto::FieldElement;

use super::pedersen;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
/// `TElement` of a merkletree
pub struct TElement(pub Vec<u8>);

impl AsRef<[u8]> for TElement {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

/// Merkle tree type
/// Leaves are hashed with pedersen hash and stored in a vector
pub type BinaryMerkle =
    merkletree::merkle::MerkleTree<TElement, pedersen::StarknetPedersenHash, VecStore<TElement>>;

/// Proof of a merkle tree leaf
pub type BinaryMerkleProof = Proof<TElement>;

/// Result type
pub type Result = std::result::Result<BinaryMerkle, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Error creating tree")]
    ErrorCreatingTree,
    #[error("Other error")]
    OtherError(#[from] anyhow::Error),
}

/// Create a merkletree given input of byte slices
/// We need atleast a vector of length 2 otherwise we cannot construct a merkle tree
pub fn create(rlps: Vec<Vec<u8>>) -> Result {
    if rlps.is_empty() {
        return Err(Error::ErrorCreatingTree);
    }

    let tree = merkletree::merkle::MerkleTree::from_data(rlps)?;
    debug!("tree: {tree:?}");

    Ok(tree)
}

impl Element for TElement {
    fn byte_len() -> usize {
        32
    }

    fn copy_to_slice(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(&self.0.as_slice()[..bytes.len()]);
    }

    fn from_slice(bytes: &[u8]) -> Self {
        TElement(Vec::from(bytes))
    }
}

impl From<TElement> for FieldElement {
    fn from(val: TElement) -> Self {
        FieldElement::from_byte_slice_be(val.as_ref()).unwrap()
    }
}

impl From<TElement> for Felt {
    fn from(tree_element: TElement) -> Felt {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(tree_element.0.as_slice());
        bytes
    }
}
