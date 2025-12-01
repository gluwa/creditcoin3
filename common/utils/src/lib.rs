#![cfg_attr(not(feature = "std"), no_std)]

//! # Creditcoin3 Utils
//!
//! This crate provides common utilities for Creditcoin3, including:
//!
//! - **Block Item Traits**: Interfaces for blockchain items with unique identifiers
//! - **Keccak Merkle Trees**: Keccak256 hash implementation for Merkle structures
//! - **Type Conversions**: Utilities for converting between types and parsing
//! - **JSON Serialization**: File-based JSON serialization traits (std only)
//!
//! ## Features
//!
//! - `std`: Enables standard library features including file I/O and JSON serialization
//! - Default features include `std`
//!
//! ## Usage
//!
//! ```rust
//! use utils::BlockItemIdentifier;
//! use merkle::KeccakMerkleTree;
//!
//! // Create a block item identifier
//! let id = BlockItemIdentifier::new(100, 5);
//!
//! // Use KeccakMerkleTree (matches POC implementation)
//! let data = vec![b"hello".to_vec(), b"world".to_vec()];
//! let tree = KeccakMerkleTree::new(&data);
//! ```

// =============================================================================
// Module Declarations
// =============================================================================

pub mod block_item_traits;

#[cfg(feature = "std")]
pub mod json_serializable;

// =============================================================================
// Re-exports
// =============================================================================

// Core traits and types
pub use block_item_traits::{BlockItem, BlockItemIdentifier};

// JSON serialization (std only)
#[cfg(feature = "std")]
pub use json_serializable::JsonSerializable;

// =============================================================================
// Crate-level Tests
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[cfg(feature = "std")]
    #[test]
    fn test_json_serializable_available() {
        // Just verify the trait is available when std is enabled
        fn _test_json_serializable<T: JsonSerializable>() {}
        _test_json_serializable::<BlockItemIdentifier>();
    }
}
