//! Keccak256 hash implementation for Merkle trees.
//!
//! This module provides a Keccak256 hash implementation that conforms to the `HashT` trait,
//! allowing it to be used with the generic Merkle tree implementation.
//! This is particularly useful for Ethereum-compatible Merkle proofs.

use crate::traits::HashT;
use core::fmt::{Debug, Display, Formatter};
use sp_core::H256;

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

/// Type alias for a Keccak256-based Merkle tree
pub type KeccakMerkleTree = crate::BaseTree<Keccak256>;

/// Type alias for a Keccak256-based Merkle proof
pub type KeccakMerkleProof = crate::proof::Proof<Keccak256>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keccak_merkle_tree() {
        // Test data
        let data: Vec<&[u8]> = vec![b"leaf1", b"leaf2", b"leaf3", b"leaf4"];

        // Create tree
        let tree = KeccakMerkleTree::from(&data[..]);

        // Generate proof for first leaf
        let proof = tree.generate_proof(0);

        // The proof should be verifiable
        assert!(proof.validate(data[0]));
    }

    #[test]
    fn test_single_leaf_tree() {
        let data: Vec<&[u8]> = vec![b"single_leaf"];
        let tree = KeccakMerkleTree::from(&data[..]);

        let proof = tree.generate_proof(0);
        assert!(proof.validate(data[0]));
    }
}
