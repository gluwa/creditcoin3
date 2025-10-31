//! Keccak256-based Merkle tree implementation
//!
//! This module provides a Keccak256 hasher for Merkle trees, replacing the Pedersen hash
//! implementation. This is more efficient and standard in Ethereum ecosystem.

use crate::block_item_traits::BlockItem;
use core::fmt;
use core::hash::{Hash, Hasher};
use mmr::{traits::HashT, BaseTree};
use sp_std::vec::Vec;

// Create a newtype wrapper for [u8; 32] to implement required traits
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub struct Hash256(pub [u8; 32]);

impl Hash256 {
    pub fn from_slice(bytes: &[u8]) -> Self {
        let mut array = [0u8; 32];
        array.copy_from_slice(bytes);
        Hash256(array)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_h256(&self) -> sp_core::H256 {
        sp_core::H256::from(self.0)
    }
}

impl From<Hash256> for sp_core::H256 {
    fn from(hash: Hash256) -> Self {
        sp_core::H256::from(hash.0)
    }
}

impl From<sp_core::H256> for Hash256 {
    fn from(h: sp_core::H256) -> Self {
        Hash256(h.to_fixed_bytes())
    }
}

impl From<u8> for Hash256 {
    fn from(byte: u8) -> Self {
        let mut result = [0u8; 32];
        result[0] = byte;
        Hash256(result)
    }
}

impl From<[u8; 32]> for Hash256 {
    fn from(bytes: [u8; 32]) -> Self {
        Hash256(bytes)
    }
}

impl fmt::Debug for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

impl Hash for Hash256 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// Keccak256 hasher for Merkle trees
#[derive(Debug, Clone, Copy)]
pub struct KeccakHasher;

/// Type alias for a Keccak256-based Merkle tree
pub type KeccakMerkleTree = BaseTree<KeccakHasher>;

impl HashT for KeccakHasher {
    type Output = Hash256;

    fn hash(input: &[u8]) -> Self::Output {
        // Use sha3 crate which supports both std and no_std
        use sha3::{Digest, Keccak256};
        let mut hasher = Keccak256::new();
        hasher.update(input);
        let result = hasher.finalize();
        Hash256::from_slice(&result)
    }
}

/// Create a Keccak256 Merkle tree from items that implement BlockItem trait
pub fn keccak_merkle_tree<T: BlockItem>(items: &[T]) -> KeccakMerkleTree {
    let bytes: Vec<Vec<u8>> = items.iter().map(|item| item.to_bytes()).collect();
    KeccakMerkleTree::from(&bytes[..])
}

/// Compute a Keccak256 hash of multiple Hash256 values (for continuity chain)
pub fn keccak_hash_h256_array(values: &[Hash256]) -> Hash256 {
    let mut bytes = Vec::with_capacity(values.len() * 32);
    for value in values {
        bytes.extend_from_slice(&value.0);
    }
    KeccakHasher::hash(&bytes)
}

/// Compute a continuity chain digest using Keccak256
/// digest = keccak256(block_number || root || prev_digest)
pub fn compute_digest(block_number: u64, root: &Hash256, prev_digest: &Hash256) -> Hash256 {
    let mut bytes = Vec::with_capacity(8 + 32 + 32);
    bytes.extend_from_slice(&block_number.to_be_bytes());
    bytes.extend_from_slice(&root.0);
    bytes.extend_from_slice(&prev_digest.0);
    KeccakHasher::hash(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keccak_hasher() {
        let data1 = b"hello";
        let data2 = b"world";

        let hash1 = KeccakHasher::hash(data1);
        let hash2 = KeccakHasher::hash(data2);

        // Verify hashes are deterministic
        assert_eq!(hash1, KeccakHasher::hash(data1));
        assert_eq!(hash2, KeccakHasher::hash(data2));

        // Verify different data produces different hashes
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_digest() {
        let block_number = 100u64;
        let root = Hash256([1u8; 32]);
        let prev_digest = Hash256([2u8; 32]);

        let digest = compute_digest(block_number, &root, &prev_digest);

        // Verify digest is deterministic
        assert_eq!(digest, compute_digest(block_number, &root, &prev_digest));

        // Verify changing inputs changes digest
        let digest2 = compute_digest(block_number + 1, &root, &prev_digest);
        assert_ne!(digest, digest2);
    }
}
