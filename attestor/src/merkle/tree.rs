use anyhow::Result;
use merkletree::{merkle::Element, proof::Proof, store::VecStore};
use starknet_crypto::FieldElement;
use std::hash::Hash;
use tracing::debug;

use super::pedersen;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct TreeElement(pub Vec<u8>);

impl AsRef<[u8]> for TreeElement {
    fn as_ref(&self) -> &[u8] {
        &self.0.as_slice()
    }
}

pub type TxRxBinaryMerkleTree = merkletree::merkle::MerkleTree<
    TreeElement,
    pedersen::StarknetPedersenHash,
    VecStore<TreeElement>,
>;

pub type TxRxBinaryMerkleProof = Proof<TreeElement>;

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

pub fn create(rlps: Vec<Vec<u8>>) -> Result<TxRxBinaryMerkleTree> {
    let tree = merkletree::merkle::MerkleTree::from_data(rlps)?;
    debug!("tree: {tree:?}");
    Ok(tree)
}

impl Into<FieldElement> for TreeElement {
    fn into(self) -> FieldElement {
        FieldElement::from_byte_slice_be(self.as_ref()).unwrap()
    }
}
