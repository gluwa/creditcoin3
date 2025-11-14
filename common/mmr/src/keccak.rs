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
        write!(f, "{:?}", self.0)
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

/// Simple Merkle tree that matches POC implementation exactly
/// This duplicates the last node when odd number of nodes at a level
#[derive(Debug, Clone)]
pub struct SimpleMerkleTree {
    /// All levels of the tree, from leaves to root
    levels: Vec<Vec<H256>>,
}

impl SimpleMerkleTree {
    /// Create a new Merkle tree from raw data items
    ///
    /// # Empty Input Handling
    /// If `items` is empty, returns a tree with a default hash root (all zeros).
    pub fn new(items: &[Vec<u8>]) -> Self {
        // Handle empty input: return tree with default hash root
        if items.is_empty() {
            return SimpleMerkleTree {
                levels: vec![vec![H256::default()]],
            };
        }

        let mut levels = Vec::new();

        // Level 0: Hash all leaves with prefix
        let leaf_hashes: Vec<H256> = items
            .iter()
            .map(|item| {
                let mut prefixed = Vec::with_capacity(item.len() + 1);
                prefixed.push(0x00); // LEAF_PREFIX
                prefixed.extend_from_slice(item);
                H256::from(sp_io::hashing::keccak_256(&prefixed))
            })
            .collect();

        levels.push(leaf_hashes.clone());

        // Build tree level by level
        let mut current_level = leaf_hashes;

        while current_level.len() > 1 {
            let mut next_level = Vec::new();

            // Process pairs
            let mut i = 0;
            while i < current_level.len() {
                let left = current_level[i];
                let right = if i + 1 < current_level.len() {
                    current_level[i + 1]
                } else {
                    // Odd number: duplicate the last node (matches POC implementation)
                    current_level[i]
                };

                // Hash inner node with prefix
                let mut prefixed = Vec::with_capacity(65);
                prefixed.push(0x01); // INNER_PREFIX
                prefixed.extend_from_slice(left.as_bytes());
                prefixed.extend_from_slice(right.as_bytes());
                let parent = H256::from(sp_io::hashing::keccak_256(&prefixed));
                next_level.push(parent);

                i += 2;
            }

            levels.push(next_level.clone());
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
        if self.levels.is_empty() || self.levels[0].is_empty() {
            panic!("Cannot generate proof for empty tree");
        }
        if leaf_index >= self.levels[0].len() {
            panic!(
                "Leaf index {leaf_index} out of range (tree has {} leaves)",
                self.levels[0].len()
            );
        }

        let mut siblings = Vec::new();
        let mut current_index = leaf_index;

        // Traverse from leaves to root (excluding the root level)
        for level in &self.levels[..self.levels.len() - 1] {
            let is_left_node = current_index % 2 == 0;
            let sibling = if is_left_node {
                // Current is left, sibling is right
                if current_index + 1 < level.len() {
                    level[current_index + 1]
                } else {
                    // No right sibling, duplicate current (matches POC)
                    level[current_index]
                }
            } else {
                // Current is right, sibling is left
                level[current_index - 1]
            };
            let is_sibling_left = !is_left_node;

            siblings.push(crate::query_proof::MerkleProofEntry {
                hash: sibling,
                is_left: is_sibling_left,
            });

            // Move to parent index
            current_index /= 2;
        }

        crate::query_proof::QueryMerkleProof::new(self.root(), siblings)
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
}
