use merkletree::{merkle::Element, proof::Proof, store::VecStore};
use std::hash::Hash;
use thiserror::Error;
use tracing::debug;

pub use starknet_crypto::FieldElement;

use super::pedersen;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
/// TreeElement of a merkletree
pub struct TreeElement(pub Vec<u8>);

impl AsRef<[u8]> for TreeElement {
    fn as_ref(&self) -> &[u8] {
        &self.0.as_slice()
    }
}

/// Merkle tree type
/// Leaves are hashed with pedersen hash and stored in a vector
pub type TxRxBinaryMerkleTree = merkletree::merkle::MerkleTree<
    TreeElement,
    pedersen::StarknetPedersenHash,
    VecStore<TreeElement>,
>;

/// Proof of a merkle tree leaf
pub type TxRxBinaryMerkleProof = Proof<TreeElement>;

/// Result type
pub type Result = std::result::Result<TxRxBinaryMerkleTree, Error>;

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
    if rlps.len() < 1 {
        return Err(Error::ErrorCreatingTree);
    }

    let tree = merkletree::merkle::MerkleTree::from_data(rlps)?;
    debug!("tree: {tree:?}");

    Ok(tree)
}

impl Element for TreeElement {
    fn byte_len() -> usize {
        32
    }

    fn copy_to_slice(&self, bytes: &mut [u8]) {
        bytes.copy_from_slice(&self.0.as_slice()[..bytes.len()]);
    }

    fn from_slice(bytes: &[u8]) -> Self {
        TreeElement(Vec::from(bytes))
    }
}

impl Into<FieldElement> for TreeElement {
    fn into(self) -> FieldElement {
        FieldElement::from_byte_slice_be(self.as_ref()).unwrap()
    }
}
