//! Merkle proof implementation for transaction inclusion verification.
//!
//! This module provides a specialized Merkle proof structure designed for
//! transaction inclusion verification. It can be used in SDKs and precompiles.

use crate::keccak::{hash_inner, hash_leaf};
use parity_scale_codec::{Decode, Encode};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use sp_std::vec::Vec;

#[cfg(feature = "precompile-support")]
use precompile_utils::prelude::String;
#[cfg(feature = "precompile-support")]
use precompile_utils::solidity::Codec;

/// Transaction inclusion Merkle proof structure
///
/// This structure maintains compatibility with the Solidity ABI while leveraging
/// Keccak256 hash functions. Used to prove that a transaction is included in a
/// block's Merkle tree.
#[derive(
    Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "precompile-support", derive(Codec))]
pub struct TransactionMerkleProof {
    /// The Merkle root hash
    pub root: H256,
    /// Sibling hashes with position information
    pub siblings: Vec<MerkleProofEntry>,
}

/// A single entry in the merkle proof
#[derive(
    Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "precompile-support", derive(Codec))]
#[serde(rename_all = "camelCase")]
pub struct MerkleProofEntry {
    /// The sibling hash
    pub hash: H256,
    /// Indicates the relative position with respect to its sibling
    pub is_left: bool,
}

impl TransactionMerkleProof {
    /// Create a new TransactionMerkleProof
    pub fn new(root: H256, siblings: Vec<MerkleProofEntry>) -> Self {
        Self { root, siblings }
    }

    /// Verify the Merkle proof for transaction inclusion using Keccak256 hash
    pub fn verify(&self, tx_data: &[u8]) -> bool {
        let mut current_hash = hash_leaf(tx_data);

        // Traverse the Merkle path using siblings with position information
        for entry in &self.siblings {
            let (left, right) = if entry.is_left {
                (entry.hash.as_bytes(), current_hash.as_bytes())
            } else {
                (current_hash.as_bytes(), entry.hash.as_bytes())
            };

            current_hash = hash_inner(left, right);
        }

        // Compare computed root with provided root
        current_hash == self.root
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

        let proof = TransactionMerkleProof::new(root, siblings.clone());

        assert_eq!(proof.root, root);
        assert_eq!(proof.siblings, siblings);
        assert_eq!(proof.levels(), 2);
        assert!(!proof.is_single_transaction());
    }

    #[test]
    fn test_single_transaction_detection() {
        let root = H256::from([1u8; 32]);
        let proof = TransactionMerkleProof::new(root, vec![]);

        assert!(proof.is_single_transaction());
        assert_eq!(proof.levels(), 0);
    }
}
