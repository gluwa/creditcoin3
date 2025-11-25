//! Keccak256 hash implementation for Merkle trees.
//!
//! This module provides a Keccak256 hash implementation that conforms to the `HashT` trait,
//! allowing it to be used with the generic Merkle tree implementation.
//! This is particularly useful for Ethereum-compatible Merkle proofs.

use crate::traits::HashT;
use core::fmt::{Debug, Display, Formatter};
use sp_core::H256;
use sp_std::{vec, vec::Vec};

/// Wrapper type for H256 that implements required traits for HashT
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct KeccakHash(pub H256);

impl From<u8> for KeccakHash {
    fn from(byte: u8) -> Self {
        let mut bytes = [0u8; 32];
        bytes[0] = byte;
        KeccakHash(H256::from(bytes))
    }
}

impl From<[u8; 32]> for KeccakHash {
    fn from(bytes: [u8; 32]) -> Self {
        KeccakHash(H256::from(bytes))
    }
}

impl From<H256> for KeccakHash {
    fn from(h: H256) -> Self {
        KeccakHash(h)
    }
}

impl Debug for KeccakHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl Display for KeccakHash {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Keccak256 hash implementation for use in Merkle trees
pub struct Keccak256;

impl HashT for Keccak256 {
    type Output = KeccakHash;

    fn hash(input: &[u8]) -> Self::Output {
        KeccakHash(H256::from(sp_io::hashing::keccak_256(input)))
    }
}

/// Type alias for a Keccak256-based Merkle proof
pub type KeccakMerkleProof = crate::proof::Proof<Keccak256>;

/// Compute a continuity chain digest using Keccak256
/// digest = keccak256(block_number || root || prev_digest)
pub fn compute_digest(block_number: u64, root: H256, prev_digest: H256) -> H256 {
    let mut bytes = Vec::with_capacity(8 + 32 + 32);
    bytes.extend_from_slice(&block_number.to_be_bytes());
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(prev_digest.as_bytes());
    H256::from(sp_io::hashing::keccak_256(&bytes))
}

#[inline]
pub fn hash_leaf(input: &[u8]) -> H256 {
    let mut prefixed = sp_std::vec![crate::LEAF_HASH_PREPEND_VALUE; input.len() + 1];

    prefixed[1..].copy_from_slice(input);

    sp_io::hashing::keccak_256(&prefixed).into()
}

#[inline]
pub fn hash_inner(left: &[u8], right: &[u8]) -> H256 {
    let mut prefixed = [crate::INNER_HASH_PREPEND_VALUE; 1 + 2 * size_of::<H256>()];

    prefixed[1..1 + size_of::<H256>()].copy_from_slice(left);
    prefixed[1 + size_of::<H256>()..1 + 2 * size_of::<H256>()].copy_from_slice(right);

    sp_io::hashing::keccak_256(&prefixed).into()
}

/// Simple Merkle tree that matches POC implementation exactly
/// This duplicates the last node when odd number of nodes at a level
#[derive(Debug, Clone)]
pub struct SimpleMerkleTree {
    /// All levels of the tree, from leaves to root
    levels: Vec<Vec<H256>>,
}

impl SimpleMerkleTree {
    const PAD_HASH: H256 = H256([0; 32]);
    /// Create a new Merkle tree from raw data items
    pub fn new(items: &[Vec<u8>]) -> Self {
        let mut levels = vec![];
        let mut current_level = items
            .iter()
            .map(|item| hash_leaf(&item[..]))
            .collect::<Vec<_>>();

        while !current_level.is_empty() {
            let current_len = current_level.len();

            let next_level = if current_len > 1 {
                (0..current_len)
                    .step_by(2)
                    .map(|i| {
                        let left = current_level[i].as_bytes();
                        let right = current_level
                            .get(i + 1)
                            .unwrap_or(&Self::PAD_HASH)
                            .as_bytes();

                        hash_inner(left, right)
                    })
                    .collect()
            } else {
                vec![]
            };

            levels.push(current_level);

            current_level = next_level;
        }

        SimpleMerkleTree { levels }
    }

    /// Get the root hash
    pub fn root(&self) -> H256 {
        self.levels
            .last()
            .and_then(|level| level.first())
            .copied()
            .unwrap_or_default()
    }

    /// Generate a Merkle proof for a specific leaf index
    ///
    /// # Panics
    /// Panics if `leaf_index` is out of range or if the tree is empty.
    pub fn generate_proof(&self, leaf_index: usize) -> crate::query_proof::QueryMerkleProof {
        let mut current_index = leaf_index;

        // Traverse from leaf to root (excluding the root level)
        let path = self
            .levels
            .iter()
            // this is needed just because for some reason it was decided to not include the root in the path
            .rev()
            .skip(1)
            .rev()
            .map(|level| {
                // sibling_offset is opposite to the item's offset which is (current_index % 2):
                // 0 -> 1
                // 1 -> 0
                let sibling_offset = 1 - (current_index % 2);
                // transform sibling offset to sibling index in the current level:
                // 0 -> current_index - 1
                // 1 -> current_index + 1
                let sibling_index = current_index + 2 * sibling_offset - 1;

                // Move to parent index
                current_index /= 2;

                crate::query_proof::MerkleProofEntry {
                    hash: *level.get(sibling_index).unwrap_or(&Self::PAD_HASH),
                    is_left: sibling_offset == 0,
                }
            })
            .collect();

        crate::query_proof::QueryMerkleProof::new(self.root(), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_merkle_tree_empty() {
        let items: Vec<Vec<u8>> = vec![];
        let tree = SimpleMerkleTree::new(&items);

        // Empty tree should have default hash root (all zeros)
        assert_eq!(tree.root(), H256::default());
    }

    #[test]
    fn test_simple_merkle_tree_single_item() {
        let items = vec![vec![0x11; 32]];
        let tree = SimpleMerkleTree::new(&items);

        // Single item tree - root should be hash of leaf
        let mut expected = Vec::with_capacity(33);
        expected.push(0x00); // LEAF_PREFIX
        expected.extend_from_slice(&items[0]);
        let expected_root = H256::from(sp_io::hashing::keccak_256(&expected));

        assert_eq!(tree.root(), expected_root);

        // Generate proof for the single item
        let proof = tree.generate_proof(0);
        assert_eq!(proof.siblings.len(), 0); // No siblings for single item
        assert_eq!(proof.root, expected_root);
    }

    #[test]
    fn test_simple_merkle_tree_two_items() {
        let items = vec![vec![0x11; 32], vec![0x22; 32]];
        let tree = SimpleMerkleTree::new(&items);

        // Generate and verify proof for first item
        let proof = tree.generate_proof(0);
        assert!(proof.verify(&items[0]));

        // Generate and verify proof for second item
        let proof = tree.generate_proof(1);
        assert!(proof.verify(&items[1]));
    }

    #[test]
    fn test_simple_merkle_tree_odd_items() {
        let items = vec![vec![0x11; 32], vec![0x22; 32], vec![0x33; 32]];
        let tree = SimpleMerkleTree::new(&items);

        // Generate and verify proof for all items
        for (i, item) in items.iter().enumerate() {
            let proof = tree.generate_proof(i);
            assert!(proof.verify(item), "Failed to verify proof for item {i}");
        }
    }

    #[test]
    fn tampered_proof_test() {
        let items = vec![vec![0x11; 32], vec![0x22; 32], vec![0x33; 32]];
        let tree = SimpleMerkleTree::new(&items);

        let mut proof = tree.generate_proof(2);

        // sibling swapped
        proof.siblings[0].is_left = !proof.siblings[0].is_left;

        assert_eq!(
            proof.verify(&items[2]),
            false,
            "Expected to fail due to tampered proof."
        );
    }
}
