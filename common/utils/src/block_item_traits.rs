//! Block item traits for Creditcoin3.
//!
//! This module defines traits for serializing block items (transactions, events, etc.)
//! within the Creditcoin3 blockchain.

use core::fmt::Debug;

use sp_std::vec::Vec;

/// Trait for items that can be stored in a block.
///
/// This trait defines the interface for any item that can be included in a block
/// and requires serialization for storage or transmission. Items implementing
/// this trait can be uniquely identified and converted to bytes for hashing.
pub trait BlockItem: Sized + Debug {
    /// Converts the entire block item to bytes for hashing or storage.
    ///
    /// The default implementation returns only the payload bytes.
    /// The identifier is not included to reduce overhead during decoding.
    ///
    /// # Returns
    /// * `Vec<u8>` - The byte representation of this item (payload only)
    fn to_bytes(&self) -> Vec<u8> {
        self.payload_bytes()
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use sp_std::vec;

    #[derive(Debug)]
    struct TestBlockItem {
        data: Vec<u8>,
        tx_type: Option<u8>,
    }

    impl BlockItem for TestBlockItem {
        fn payload_bytes(&self) -> Vec<u8> {
            self.data.clone()
        }

        fn tx_type(&self) -> Option<u8> {
            self.tx_type
        }
    }

    #[test]
    fn test_block_item_trait() {
        let item = TestBlockItem {
            data: vec![1, 2, 3, 4],
            tx_type: Some(42),
        };

        assert_eq!(item.payload_bytes(), vec![1, 2, 3, 4]);
        assert_eq!(item.tx_type(), Some(42));

        let bytes = item.to_bytes();
        let expected_len = 4; // payload only (4 bytes)
        assert_eq!(bytes.len(), expected_len);
        assert_eq!(item.size(), expected_len);
    }
}
