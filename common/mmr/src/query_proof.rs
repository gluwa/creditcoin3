//! Query-specific Merkle proof implementation.
//!
//! This module provides a specialized Merkle proof structure designed for
//! query verification in the native query verifier precompile. It wraps the
//! generic MMR proof implementation with a query-specific interface.

use crate::keccak::Keccak256;
use crate::traits::HashT;
use crate::{ARITY, INNER_HASH_PREPEND_VALUE, LEAF_HASH_PREPEND_VALUE};
use sp_core::H256;
use sp_std::vec::Vec;

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(all(not(feature = "std"), feature = "verification"))]
use alloc::string::String;
#[cfg(all(feature = "std", feature = "verification"))]
use std::string::String;

#[cfg(feature = "verification")]
use fp_evm::PrecompileFailure;
#[cfg(feature = "verification")]
use precompile_utils::solidity::Codec;

/// Trait for types that have an index field
#[cfg(feature = "verification")]
pub trait QueryIndex {
    fn index(&self) -> u64;
}

/// Query-specific Merkle proof structure for precompile compatibility
///
/// This structure maintains compatibility with the Solidity ABI while leveraging
/// the generic MMR proof implementation internally.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "verification", derive(Codec))]
pub struct QueryMerkleProof {
    /// The Merkle root hash
    pub root: H256,
    /// Sibling hashes for each level of the tree
    pub siblings: Vec<H256>,
}

impl QueryMerkleProof {
    /// Create a new QueryMerkleProof
    pub fn new(root: H256, siblings: Vec<H256>) -> Self {
        Self { root, siblings }
    }

    /// Verify the Merkle proof for transaction inclusion using Keccak256 hash
    ///
    /// This implements the MMR Merkle tree verification with:
    /// 1. Leaf hashing: prepend LEAF_HASH_PREFIX (0x00) to tx_data and hash with Keccak256
    /// 2. Inner node hashing: prepend INNER_HASH_PREFIX (0x01) to concatenated children and hash with Keccak256
    /// 3. Tree traversal: use the index to determine sibling positions in the binary tree
    pub fn verify(&self, tx_data: &[u8], index: u64) -> bool {
        // Step 1: Hash the transaction data as a leaf node
        // Prepend LEAF_HASH_PREFIX to tx_data before hashing
        let mut prefixed_leaf = sp_std::vec![0u8; tx_data.len() + 1];
        prefixed_leaf[0] = LEAF_HASH_PREPEND_VALUE;
        prefixed_leaf[1..].copy_from_slice(tx_data);

        let current_hash = Keccak256::hash(&prefixed_leaf);

        // Step 2: Handle single-transaction case (no siblings)
        if self.siblings.is_empty() {
            let result = current_hash.0 == self.root;
            return result;
        }

        // Step 3: Traverse the Merkle tree using siblings
        // Each level has ARITY siblings that represent the complete set of child hashes
        let mut current_hash = current_hash;
        let mut index = index;

        let siblings_per_level = ARITY; // We have ARITY hashes per level (including placeholder)
        let num_levels = self.siblings.len() / siblings_per_level;

        // Process each level of the tree
        // siblings contains ALL hashes per level (ARITY hashes including placeholder at offset)
        for level in 0..num_levels {
            let start = level * siblings_per_level;
            let end = start + siblings_per_level;

            if end > self.siblings.len() {
                // Level would exceed siblings array bounds
                break;
            }

            // Get all hashes for this level (including placeholder at offset)
            let level_siblings = &self.siblings[start..end];

            // Determine which position our current hash occupies
            let offset = (index % ARITY as u64) as usize;

            // Build the hash input with inner node prefix
            let mut hash_input = sp_std::vec![INNER_HASH_PREPEND_VALUE];

            // Add child hashes in order, replacing placeholder with current_hash
            for (i, sibling) in level_siblings.iter().enumerate().take(ARITY) {
                if i == offset {
                    // Replace placeholder with our computed hash at this position
                    hash_input.extend_from_slice(&current_hash.0 .0);
                } else {
                    // Use the hash from the proof at this position
                    hash_input.extend_from_slice(&sibling.0);
                }
            }

            // Hash with Keccak256
            current_hash = Keccak256::hash(&hash_input);

            // Move up the tree
            index /= ARITY as u64;
        }

        // Step 4: Compare computed root with provided root
        current_hash.0 == self.root
    }

    /// Get the number of levels in the Merkle tree based on siblings
    pub fn levels(&self) -> usize {
        if self.siblings.is_empty() {
            0
        } else {
            self.siblings.len() / ARITY
        }
    }

    /// Check if this is a single-transaction proof (no siblings)
    pub fn is_single_transaction(&self) -> bool {
        self.siblings.is_empty()
    }

    /// Verify with Query struct (only available with verification feature)
    /// Takes any type that has an `index` field of type u64
    #[cfg(feature = "verification")]
    pub fn verify_with_query<Q>(&self, tx_data: &[u8], query: &Q) -> Result<bool, PrecompileFailure>
    where
        Q: QueryIndex,
    {
        Ok(self.verify(tx_data, query.index()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_proof_creation() {
        let root = H256::from([1u8; 32]);
        let siblings = vec![H256::from([2u8; 32]), H256::from([3u8; 32])];

        let proof = QueryMerkleProof::new(root, siblings.clone());

        assert_eq!(proof.root, root);
        assert_eq!(proof.siblings, siblings);
        assert_eq!(proof.levels(), 1);
        assert!(!proof.is_single_transaction());
    }

    #[test]
    fn test_single_transaction_detection() {
        let root = H256::from([1u8; 32]);
        let proof = QueryMerkleProof::new(root, vec![]);

        assert!(proof.is_single_transaction());
        assert_eq!(proof.levels(), 0);
    }
}
