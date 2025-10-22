//! Block item traits and identifiers for Creditcoin3.
//!
//! This module defines traits and structures for identifying and serializing
//! block items (transactions, events, etc.) within the Creditcoin3 blockchain.

use core::fmt::Debug;
use serde::{Deserialize, Serialize};

use sp_std::{vec, vec::Vec};

use crate::utils::U248_BYTE_COUNT;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

#[cfg(feature = "std")]
use crate::json_serializable::JsonSerializable;

/// Unique identifier for items within a block.
///
/// This structure provides a way to uniquely identify any item (transaction, event, etc.)
/// within the blockchain by combining the block number and the item's index within that block.
///
/// # Fields
/// * `block_number` - The block number containing this item
/// * `index` - The position of this item within the block
#[derive(
    Debug,
    Default,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    Encode,
    Decode,
    TypeInfo,
    MaxEncodedLen,
)]
pub struct BlockItemIdentifier {
    pub block_number: u64,
    pub index: u64,
}

impl BlockItemIdentifier {
    /// Creates a new block item identifier.
    ///
    /// # Arguments
    /// * `block_number` - The block number
    /// * `index` - The index within the block
    ///
    /// # Returns
    /// * `Self` - A new BlockItemIdentifier instance
    ///
    /// # Example
    /// ```
    /// let id = BlockItemIdentifier::new(100, 5);
    /// assert_eq!(id.block_number(), 100);
    /// assert_eq!(id.index(), 5);
    /// ```
    pub const fn new(block_number: u64, index: u64) -> Self {
        Self {
            block_number,
            index,
        }
    }

    /// Returns the block number.
    ///
    /// # Returns
    /// * `u64` - The block number containing this item
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    /// Returns the index within the block.
    ///
    /// # Returns
    /// * `u64` - The position of this item within its block
    #[inline(always)]
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Converts the identifier to a byte representation suitable for hashing.
    ///
    /// The byte layout is designed to be compatible with Felt encoding:
    /// - Pads the index to fit within the 31-byte U248 constraint
    /// - Uses big-endian encoding for consistent ordering
    ///
    /// # Returns
    /// * `Vec<u8>` - The byte representation of this identifier
    ///
    /// # Memory Layout
    /// ```text
    /// [padding: 23 bytes][index: 8 bytes (big-endian)]
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        use core::mem::size_of;

        // Calculate padding needed to align index properly within U248 constraint
        const INDEX_PADDING_LEN: usize = U248_BYTE_COUNT - size_of::<u64>();

        // Total buffer size: padding + index
        let mut buffer = vec![0u8; INDEX_PADDING_LEN + size_of::<u64>()];

        // Place index at the end in big-endian format
        let index_offset = INDEX_PADDING_LEN;
        buffer[index_offset..].copy_from_slice(&self.index.to_be_bytes());

        buffer
    }
}

impl core::fmt::Display for BlockItemIdentifier {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}:{}", self.block_number, self.index)
    }
}

/// Trait for items that can be stored in a block.
///
/// This trait defines the interface for any item that can be included in a block
/// and requires serialization for storage or transmission. Items implementing
/// this trait can be uniquely identified and converted to bytes for hashing.
pub trait BlockItem: Sized + Debug {
    /// Converts the entire block item to bytes for hashing or storage.
    ///
    /// The default implementation combines the identifier bytes with the payload bytes.
    /// This ensures that the item's position in the block is included in any hash.
    ///
    /// # Returns
    /// * `Vec<u8>` - The complete byte representation of this item
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.id().to_bytes();
        bytes.extend(self.payload_bytes());
        bytes
    }

    /// Returns the unique identifier for this block item.
    ///
    /// # Returns
    /// * `&BlockItemIdentifier` - Reference to this item's identifier
    fn id(&self) -> &BlockItemIdentifier;

    /// Returns the payload-specific bytes for this item.
    ///
    /// This method should return the bytes that represent the actual content
    /// of the item, excluding the identifier information.
    ///
    /// # Returns
    /// * `Vec<u8>` - The payload bytes
    fn payload_bytes(&self) -> Vec<u8>;

    /// Returns the transaction type identifier, if applicable.
    ///
    /// This is used for items that represent different types of transactions
    /// or operations within the blockchain.
    ///
    /// # Returns
    /// * `Option<u8>` - The transaction type, or None if not applicable
    fn tx_type(&self) -> Option<u8> {
        None
    }

    /// Returns the size of this item in bytes.
    ///
    /// This is useful for calculating block sizes and fees.
    ///
    /// # Returns
    /// * `usize` - The total size of this item in bytes
    fn size(&self) -> usize {
        self.to_bytes().len()
    }
}

#[cfg(feature = "std")]
impl JsonSerializable for BlockItemIdentifier {}

#[cfg(test)]
mod tests {
    use super::*;
    use sp_std::vec;

    #[derive(Debug)]
    struct TestBlockItem {
        id: BlockItemIdentifier,
        data: Vec<u8>,
        tx_type: Option<u8>,
    }

    impl BlockItem for TestBlockItem {
        fn id(&self) -> &BlockItemIdentifier {
            &self.id
        }

        fn payload_bytes(&self) -> Vec<u8> {
            self.data.clone()
        }

        fn tx_type(&self) -> Option<u8> {
            self.tx_type
        }
    }

    #[test]
    fn test_block_item_identifier_creation() {
        let id = BlockItemIdentifier::new(42, 100);
        assert_eq!(id.block_number(), 42);
        assert_eq!(id.index(), 100);
    }

    #[test]
    fn test_block_item_identifier_display() {
        let id = BlockItemIdentifier::new(123, 456);
        assert_eq!(format!("{id}"), "123:456");
    }

    #[test]
    fn test_block_item_identifier_ordering() {
        let id1 = BlockItemIdentifier::new(1, 1);
        let id2 = BlockItemIdentifier::new(1, 2);
        let id3 = BlockItemIdentifier::new(2, 1);

        assert!(id1 < id2);
        assert!(id2 < id3);
        assert!(id1 < id3);
    }

    #[test]
    fn test_block_item_identifier_to_bytes() {
        let id = BlockItemIdentifier::new(42, 100);
        let bytes = id.to_bytes();

        // Should have correct length
        assert_eq!(bytes.len(), U248_BYTE_COUNT);

        // Last 8 bytes should be the index in big-endian
        let index_bytes = &bytes[bytes.len() - 8..];
        assert_eq!(index_bytes, &100u64.to_be_bytes());

        // Earlier bytes should be zero padding
        let padding = &bytes[..bytes.len() - 8];
        assert!(padding.iter().all(|&b| b == 0));
    }

    #[test]
    fn test_block_item_trait() {
        let item = TestBlockItem {
            id: BlockItemIdentifier::new(10, 5),
            data: vec![1, 2, 3, 4],
            tx_type: Some(42),
        };

        assert_eq!(item.id().block_number(), 10);
        assert_eq!(item.id().index(), 5);
        assert_eq!(item.payload_bytes(), vec![1, 2, 3, 4]);
        assert_eq!(item.tx_type(), Some(42));

        let bytes = item.to_bytes();
        let expected_len = U248_BYTE_COUNT + 4; // identifier + payload
        assert_eq!(bytes.len(), expected_len);
        assert_eq!(item.size(), expected_len);
    }

    #[test]
    fn test_serialization() {
        let id = BlockItemIdentifier::new(12345, 67890);

        // Test JSON serialization
        let json = serde_json::to_string(&id).expect("Failed to serialize to JSON");
        let deserialized: BlockItemIdentifier =
            serde_json::from_str(&json).expect("Failed to deserialize from JSON");
        assert_eq!(id, deserialized);

        // Test SCALE encoding
        let encoded = id.encode();
        let decoded =
            BlockItemIdentifier::decode(&mut &encoded[..]).expect("Failed to decode SCALE");
        assert_eq!(id, decoded);
    }

    #[test]
    fn test_default() {
        let default_id = BlockItemIdentifier::default();
        assert_eq!(default_id.block_number(), 0);
        assert_eq!(default_id.index(), 0);
    }
}
