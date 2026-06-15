//! Merkle tree implementation for transaction proofs.
//!
//! This crate provides Keccak256-based Merkle tree functionality for generating and verifying
//! transaction inclusion proofs. It can be used in SDKs, precompiles, and other contexts.

#![cfg_attr(not(feature = "std"), no_std)]

pub mod keccak;
pub mod keccak_merkle_tree;
pub mod proof;

// Re-export main types for convenience
pub use keccak_merkle_tree::{KeccakMerkleTree, MerkleTreeError};
pub use proof::{MerkleProofEntry, TransactionMerkleProof};

/// Leaves will be prepended with this value prior to hashing
pub const LEAF_HASH_PREPEND_VALUE: u8 = 0;
/// Inner nodes will be prepended with this value prior to hashing
pub const INNER_HASH_PREPEND_VALUE: u8 = 1;
