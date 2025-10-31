//! Simple Keccak256 Merkle Tree Implementation
//!
//! This module provides a simple, standard Ethereum-compatible Merkle tree
//! implementation that matches the POC TypeScript/Solidity implementation exactly.
//!
//! Key features:
//! - Binary tree with Keccak256 hashing
//! - Leaf nodes prefixed with 0x00
//! - Inner nodes prefixed with 0x01
//! - Duplicates last node when odd number of nodes at a level
//! - Compatible with standard Ethereum Merkle proof verification

use sha3::{Digest, Keccak256};
use sp_core::H256;
#[cfg(test)]
use sp_std::vec;
use sp_std::vec::Vec;

/// Prefix for leaf node hashing
const LEAF_PREFIX: u8 = 0x00;

/// Prefix for inner node hashing
const INNER_PREFIX: u8 = 0x01;

/// Hash a leaf node: keccak256(0x00 || data)
pub fn hash_leaf(data: &[u8]) -> H256 {
    let mut hasher = Keccak256::new();
    hasher.update([LEAF_PREFIX]);
    hasher.update(data);
    let result = hasher.finalize();
    H256::from_slice(&result)
}

/// Hash an inner node: keccak256(0x01 || left || right)
pub fn hash_inner(left: H256, right: H256) -> H256 {
    let mut hasher = Keccak256::new();
    hasher.update([INNER_PREFIX]);
    hasher.update(left.as_bytes());
    hasher.update(right.as_bytes());
    let result = hasher.finalize();
    H256::from_slice(&result)
}

/// Simple Merkle tree structure
#[derive(Debug, Clone)]
pub struct SimpleMerkleTree {
    /// All levels of the tree, from leaves to root
    /// levels[0] = leaf hashes
    /// levels[levels.len()-1] = root (single element)
    levels: Vec<Vec<H256>>,
}

/// Merkle proof for a specific leaf
#[derive(Debug, Clone)]
pub struct MerkleProof {
    /// Sibling hashes needed for verification (from leaf to root)
    pub siblings: Vec<H256>,
    /// For each sibling, whether it's on the left (true) or right (false)
    pub is_left: Vec<bool>,
}

impl SimpleMerkleTree {
    /// Create a new Merkle tree from raw data items
    pub fn new(items: &[Vec<u8>]) -> Self {
        if items.is_empty() {
            panic!("Cannot create Merkle tree from empty array");
        }

        let mut levels = Vec::new();

        // Level 0: Hash all leaves with prefix
        let leaf_hashes: Vec<H256> = items.iter().map(|item| hash_leaf(item)).collect();

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
                    // Odd number: duplicate the last node
                    current_level[i]
                };

                let parent = hash_inner(left, right);
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
    pub fn generate_proof(&self, leaf_index: usize) -> MerkleProof {
        if self.levels.is_empty() || leaf_index >= self.levels[0].len() {
            panic!("Leaf index out of range");
        }

        let mut siblings = Vec::new();
        let mut is_left = Vec::new();
        let mut current_index = leaf_index;

        // Traverse from leaves to root (excluding the root level)
        for level in &self.levels[..self.levels.len() - 1] {
            let is_left_node = current_index % 2 == 0;
            let sibling_index = if is_left_node {
                // Current is left, sibling is right
                if current_index + 1 < level.len() {
                    current_index + 1
                } else {
                    // No right sibling, duplicate current (odd case)
                    current_index
                }
            } else {
                // Current is right, sibling is left
                current_index - 1
            };

            siblings.push(level[sibling_index]);
            // Sibling position is opposite of current position
            is_left.push(!is_left_node);

            // Move to parent index
            current_index /= 2;
        }

        MerkleProof { siblings, is_left }
    }

    /// Get the number of leaves
    pub fn num_leaves(&self) -> usize {
        self.levels.first().map(|l| l.len()).unwrap_or(0)
    }

    /// Get the height of the tree (number of levels)
    pub fn height(&self) -> usize {
        self.levels.len()
    }
}

/// Verify a Merkle proof
pub fn verify_proof(leaf_data: &[u8], proof: &MerkleProof, expected_root: H256) -> bool {
    if proof.siblings.len() != proof.is_left.len() {
        return false;
    }

    let mut current_hash = hash_leaf(leaf_data);

    for (sibling, is_sibling_left) in proof.siblings.iter().zip(proof.is_left.iter()) {
        current_hash = if *is_sibling_left {
            // Sibling is on the left
            hash_inner(*sibling, current_hash)
        } else {
            // Sibling is on the right
            hash_inner(current_hash, *sibling)
        };
    }

    current_hash == expected_root
}

/// Convert the proof to the format expected by the precompile
/// Returns a flat array of siblings with placeholders
pub fn proof_to_precompile_format(proof: &MerkleProof, leaf_index: usize) -> Vec<H256> {
    let mut siblings = Vec::new();
    let mut current_index = leaf_index;

    for (sibling, _is_sibling_left) in proof.siblings.iter().zip(proof.is_left.iter()) {
        let is_current_left = current_index % 2 == 0;

        if is_current_left {
            // Current is at position 0, sibling at position 1
            siblings.push(H256::default()); // Placeholder for current
            siblings.push(*sibling);
        } else {
            // Current is at position 1, sibling at position 0
            siblings.push(*sibling);
            siblings.push(H256::default()); // Placeholder for current
        }

        current_index /= 2;
    }

    siblings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_leaf() {
        let items = vec![vec![1, 2, 3]];
        let tree = SimpleMerkleTree::new(&items);

        // Single leaf: root should be hash of the leaf
        let expected_root = hash_leaf(&[1, 2, 3]);
        assert_eq!(tree.root(), expected_root);

        // Proof should be empty for single leaf
        let proof = tree.generate_proof(0);
        assert!(proof.siblings.is_empty());
    }

    #[test]
    fn test_two_leaves() {
        let items = vec![vec![1, 2, 3], vec![4, 5, 6]];
        let tree = SimpleMerkleTree::new(&items);

        // Root should be hash of two leaf hashes
        let leaf0 = hash_leaf(&[1, 2, 3]);
        let leaf1 = hash_leaf(&[4, 5, 6]);
        let expected_root = hash_inner(leaf0, leaf1);
        assert_eq!(tree.root(), expected_root);

        // Verify proof for leaf 0
        let proof = tree.generate_proof(0);
        assert!(verify_proof(&[1, 2, 3], &proof, tree.root()));

        // Verify proof for leaf 1
        let proof = tree.generate_proof(1);
        assert!(verify_proof(&[4, 5, 6], &proof, tree.root()));
    }

    #[test]
    fn test_odd_number_of_leaves() {
        let items = vec![vec![1], vec![2], vec![3]];
        let tree = SimpleMerkleTree::new(&items);

        // With 3 leaves, the last one should be duplicated
        let leaf0 = hash_leaf(&[1]);
        let leaf1 = hash_leaf(&[2]);
        let leaf2 = hash_leaf(&[3]);

        let parent0 = hash_inner(leaf0, leaf1);
        let parent1 = hash_inner(leaf2, leaf2); // Duplicated
        let root = hash_inner(parent0, parent1);

        assert_eq!(tree.root(), root);

        // Verify all proofs
        for i in 0..3 {
            let proof = tree.generate_proof(i);
            assert!(verify_proof(&[i as u8 + 1], &proof, tree.root()));
        }
    }

    #[test]
    fn test_proof_format_conversion() {
        let items = vec![vec![1], vec![2], vec![3], vec![4]];
        let tree = SimpleMerkleTree::new(&items);

        let proof = tree.generate_proof(2); // Get proof for index 2
        let precompile_format = proof_to_precompile_format(&proof, 2);

        // Should have 2 entries per level (2 levels for 4 leaves)
        assert_eq!(precompile_format.len(), 4);

        // Index 2 is at position 0 in its pair at level 0
        assert_eq!(precompile_format[0], H256::default()); // Placeholder
        assert_ne!(precompile_format[1], H256::default()); // Sibling
    }
}
