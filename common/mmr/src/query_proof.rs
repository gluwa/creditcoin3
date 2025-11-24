//! Query-specific Merkle proof implementation.
//!
//! This module provides a specialized Merkle proof structure designed for
//! query verification in the native query verifier precompile. It wraps the
//! generic MMR proof implementation with a query-specific interface.

use crate::keccak::Keccak256;
use crate::traits::HashT;
use crate::{INNER_HASH_PREPEND_VALUE, LEAF_HASH_PREPEND_VALUE};
use parity_scale_codec::{Decode, Encode};
use precompile_utils::{prelude::String, solidity::Codec};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_std::vec::Vec;

/// Query-specific Merkle proof structure for precompile compatibility
///
/// This structure maintains compatibility with the Solidity ABI while leveraging
/// the generic MMR proof implementation internally.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    TypeInfo,
    Decode,
    Encode,
    Hash,
    Codec,
    Default,
    Serialize,
    Deserialize,
)]
pub struct QueryMerkleProof {
    /// The Merkle root hash
    pub root: H256,
    /// Sibling hashes with position information
    pub siblings: Vec<MerkleProofEntry>,
}

/// A single entry in the merkle proof
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    TypeInfo,
    Decode,
    Encode,
    Hash,
    Codec,
    Default,
    Serialize,
    Deserialize,
)]
pub struct MerkleProofEntry {
    /// The sibling hash
    pub hash: H256,
    /// Whether this sibling is on the left (true) or right (false)
    pub is_left: bool,
}

impl QueryMerkleProof {
    /// Create a new QueryMerkleProof
    pub fn new(root: H256, siblings: Vec<MerkleProofEntry>) -> Self {
        Self { root, siblings }
    }

    /// Verify the Merkle proof for transaction inclusion using Keccak256 hash
    ///
    /// This implements the MMR Merkle tree verification with:
    /// 1. Leaf hashing: prepend LEAF_HASH_PREFIX (0x00) to tx_data and hash with Keccak256
    /// 2. Inner node hashing: prepend INNER_HASH_PREFIX (0x01) to concatenated children and hash with Keccak256
    /// 3. Tree traversal: use sibling position information (no index needed)
    pub fn verify(&self, tx_data: &[u8]) -> bool {
        // Step 1: Hash the transaction data as a leaf node
        // Prepend LEAF_HASH_PREFIX to tx_data before hashing
        let mut prefixed_leaf = sp_std::vec![0u8; tx_data.len() + 1];
        prefixed_leaf[0] = LEAF_HASH_PREPEND_VALUE;
        prefixed_leaf[1..].copy_from_slice(tx_data);

        let mut current_hash = Keccak256::hash(&prefixed_leaf);

        // Step 2: Handle single-transaction case (no siblings)
        if self.siblings.is_empty() {
            let result = current_hash.0 == self.root;
            return result;
        }

        // Step 3: Traverse the Merkle tree using siblings with position information
        for entry in &self.siblings {
            // Build the hash input with inner node prefix
            let mut hash_input = sp_std::vec![INNER_HASH_PREPEND_VALUE];

            if entry.is_left {
                // Sibling is on the left, current hash on the right
                hash_input.extend_from_slice(entry.hash.as_bytes());
                hash_input.extend_from_slice(current_hash.0.as_bytes());
            } else {
                // Current hash on the left, sibling on the right
                hash_input.extend_from_slice(current_hash.0.as_bytes());
                hash_input.extend_from_slice(entry.hash.as_bytes());
            }

            // Hash with Keccak256
            current_hash = Keccak256::hash(&hash_input);
        }

        // Step 4: Compare computed root with provided root
        current_hash.0 == self.root
    }

    /// Get the number of levels in the Merkle tree based on siblings
    pub fn levels(&self) -> usize {
        self.siblings.len()
    }

    /// Check if this is a single-transaction proof (no siblings)
    pub fn is_single_transaction(&self) -> bool {
        self.siblings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_proof_creation() {
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: false,
            },
        ];

        let proof = QueryMerkleProof::new(root, siblings.clone());

        assert_eq!(proof.root, root);
        assert_eq!(proof.siblings, siblings);
        assert_eq!(proof.levels(), 2);
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
