//! Keccak256 hash functions for Merkle tree operations.

use core::mem::size_of;
use sp_core::H256;

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
