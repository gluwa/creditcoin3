//! Keccak256-based Merkle tree implementation for transaction proofs.
//!
//! This module provides a Merkle tree implementation that matches the POC implementation
//! exactly. It uses Keccak256 hashing and is specifically designed for generating
//! transaction inclusion proofs.

use sp_core::H256;
use sp_std::{vec, vec::Vec};

use crate::keccak::{hash_inner, hash_leaf};
use crate::proof::{MerkleProofEntry, TransactionMerkleProof};

/// Keccak256-based Merkle tree that matches POC implementation exactly
/// This duplicates the last node when odd number of nodes at a level
#[derive(Debug, Clone)]
pub struct KeccakMerkleTree {
    /// All levels of the tree, from leaves to root
    levels: Vec<Vec<H256>>,
}

impl KeccakMerkleTree {
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

        KeccakMerkleTree { levels }
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
    pub fn generate_proof(&self, leaf_index: usize) -> TransactionMerkleProof {
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

                MerkleProofEntry {
                    hash: *level.get(sibling_index).unwrap_or(&Self::PAD_HASH),
                    is_left: sibling_offset == 0,
                }
            })
            .collect();

        TransactionMerkleProof::new(self.root(), path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_tree() {
        let items: Vec<Vec<u8>> = vec![];
        let tree = KeccakMerkleTree::new(&items);

        // Empty tree should have default hash root (all zeros)
        assert_eq!(tree.root(), H256::default());
    }

    #[test]
    fn test_single_item_tree() {
        let items = vec![vec![1, 2, 3]];
        let tree = KeccakMerkleTree::new(&items);

        // Single item tree - root should be hash of leaf
        let mut expected = Vec::with_capacity(33);
        expected.push(0u8); // LEAF_HASH_PREPEND_VALUE
        expected.extend_from_slice(&items[0]);

        let expected_root = H256::from(sp_io::hashing::keccak_256(&expected));

        assert_eq!(tree.root(), expected_root);

        let proof = tree.generate_proof(0);
        assert_eq!(proof.siblings.len(), 0); // No siblings for single item
        assert_eq!(proof.root, expected_root);
    }

    #[test]
    fn test_two_item_tree() {
        let items = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let tree = KeccakMerkleTree::new(&items);

        // Generate and verify proof for first item
        let proof = tree.generate_proof(0);
        assert!(proof.verify(&items[0]));

        // Generate and verify proof for second item
        let proof = tree.generate_proof(1);
        assert!(proof.verify(&items[1]));
    }

    #[test]
    fn test_multiple_items_tree() {
        let items = vec![
            vec![1, 2, 3],
            vec![4, 5, 6],
            vec![7, 8, 9],
            vec![10, 11, 12],
        ];
        let tree = KeccakMerkleTree::new(&items);

        // Generate and verify proof for all items
        for (i, item) in items.iter().enumerate() {
            let proof = tree.generate_proof(i);
            assert!(proof.verify(item), "Failed to verify proof for item {i}");
        }
    }

    #[test]
    fn test_proof_verification_failure() {
        let items = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let tree = KeccakMerkleTree::new(&items);

        let mut proof = tree.generate_proof(2);

        // sibling swapped
        proof.siblings[0].is_left = !proof.siblings[0].is_left;

        // Verification should fail
        assert!(!proof.verify(&items[2]));
    }
}
