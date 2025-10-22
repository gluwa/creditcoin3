#![cfg_attr(not(feature = "std"), no_std)]

//! # Creditcoin3 Utils
//!
//! This crate provides common utilities for Creditcoin3, including:
//!
//! - **Block Item Traits**: Interfaces for blockchain items with unique identifiers
//! - **Starknet Integration**: Pedersen hash implementation for MMR structures
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
//! use utils::{BlockItemIdentifier, Felt, felts_from_bytes};
//!
//! // Create a block item identifier
//! let id = BlockItemIdentifier::new(100, 5);
//!
//! // Use Starknet types
//! let felt = Felt::from(42u64);
//!
//! // Convert bytes to felts
//! let data = b"hello";
//! let felts = felts_from_bytes(data);
//! ```

// =============================================================================
// Module Declarations
// =============================================================================

pub mod block_item_traits;
pub mod pedersen_hash;
pub mod utils;

#[cfg(feature = "std")]
pub mod json_serializable;

// =============================================================================
// Re-exports
// =============================================================================

// Core traits and types
pub use block_item_traits::{BlockItem, BlockItemIdentifier};
pub use pedersen_hash::StarknetPedersenHash;

// JSON serialization (std only)
#[cfg(feature = "std")]
pub use json_serializable::JsonSerializable;

// Utility functions - only the ones actually used in the codebase
pub use utils::{
    // Byte/Felt conversions
    felts_from_bytes,
    felts_to_bytes,
    // Parsing utilities
    try_parse_felt,
    try_parse_u64,
    try_parse_usize,
    // Constants
    U248_BYTE_COUNT,
};

// Pedersen hash function
pub use pedersen_hash::pedersen_array;

// =============================================================================
// Type Aliases
// =============================================================================

/// Starknet field element type
pub type Felt = starknet_crypto::Felt;

/// Merkle tree using Starknet Pedersen hash
pub type StarknetPedersenMerkleTree = mmr::BaseTree<StarknetPedersenHash>;

/// Merkle proof using Starknet Pedersen hash
pub type StarknetPedersenMerkleProof = mmr::proof::Proof<StarknetPedersenHash>;

// =============================================================================
// Crate-level Tests
// =============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn test_basic_type_usage() {
        // Test basic Felt usage
        let felt = Felt::from(42u64);
        assert_eq!(felt, Felt::from(42u64));

        // Test BlockItemIdentifier
        let id = BlockItemIdentifier::new(100, 5);
        assert_eq!(id.block_number(), 100);
        assert_eq!(id.index(), 5);
    }

    #[test]
    fn test_felt_conversion_integration() {
        let original_bytes = vec![1, 2, 3, 4, 5];
        let felts = felts_from_bytes(&original_bytes);
        let reconstructed = felts_to_bytes(&felts, Some(original_bytes.len()));
        assert_eq!(original_bytes, reconstructed);
    }

    #[test]
    fn test_parsing_integration() {
        assert_eq!(try_parse_u64("123").unwrap(), 123);
        assert_eq!(try_parse_u64("0x7B").unwrap(), 123);

        assert_eq!(try_parse_felt("42").unwrap(), Felt::from(42u64));
        assert_eq!(try_parse_felt("0x2A").unwrap(), Felt::from(42u64));
    }

    #[cfg(feature = "std")]
    #[test]
    fn test_json_serializable_available() {
        // Just verify the trait is available when std is enabled
        fn _test_json_serializable<T: JsonSerializable>() {}
        _test_json_serializable::<BlockItemIdentifier>();
    }
}
