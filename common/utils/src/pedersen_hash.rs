//! Starknet Pedersen hash implementation for MMR usage.
//!
//! This module provides an implementation of the `HashT` trait from the MMR crate
//! using Starknet's Pedersen hash function, enabling its use in Merkle trees and
//! MMR structures.

use crate::utils::felts_from_bytes;
use core::fmt::Debug;
use starknet_crypto::{pedersen_hash, Felt};

/// Starknet Pedersen hash implementation that can be used with MMR structures.
///
/// This struct implements the `HashT` trait from the MMR crate, allowing
/// Starknet's Pedersen hash to be used as the hashing function in Merkle
/// trees and MMR data structures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarknetPedersenHash;

impl mmr::traits::HashT for StarknetPedersenHash {
    type Output = Felt;

    /// Hashes arbitrary byte data using Starknet Pedersen hash.
    ///
    /// The input bytes are first converted to Felt elements using 31-byte chunks,
    /// then hashed using the Pedersen array hash function.
    ///
    /// # Arguments
    /// * `data` - The byte data to hash
    ///
    /// # Returns
    /// * `Self::Output` - The resulting hash as a Felt
    fn hash(data: &[u8]) -> Self::Output {
        let felts = felts_from_bytes(data);
        pedersen_array(&felts)
    }
}

/// Computes Pedersen hash of an array of Felt values.
///
/// This function implements a specific array hashing scheme:
/// 1. Start with the first element as the accumulator
/// 2. For each subsequent element, compute `pedersen_hash(accumulator, element)`
/// 3. Finally, hash the result with the array length
///
/// # Arguments
/// * `felts` - Array of values that can be converted to Felt references
///
/// # Returns
/// * `Felt` - The final hash value
///
/// # Examples
/// ```
/// use utils::pedersen_hash::pedersen_array;
/// use utils::Felt;
///
/// let elements = [Felt::from(1u64), Felt::from(2u64)];
/// let hash = pedersen_array(&elements);
/// ```
pub fn pedersen_array<T: AsRef<Felt>>(felts: &[T]) -> Felt {
    if felts.is_empty() {
        return Felt::ZERO;
    }

    let mut accumulator = *felts[0].as_ref();

    // Hash each subsequent element with the accumulator
    for felt in &felts[1..] {
        accumulator = pedersen_hash(&accumulator, felt.as_ref());
    }

    // Include array length in the final hash
    let length_felt = Felt::from(felts.len());
    pedersen_hash(&accumulator, &length_felt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmr::traits::HashT;

    #[test]
    fn test_pedersen_hash_two_elements() {
        let a = Felt::from(1u64);
        let b = Felt::from(2u64);

        let result = pedersen_hash(&a, &b);

        // Expected result from reference implementation
        assert_eq!(
            result.to_bytes_be(),
            hex::decode("05bb9440e27889a364bcb678b1f679ecd1347acdedcbf36e83494f857cc58026")
                .unwrap()
                .as_slice()
        );
    }

    #[test]
    fn test_pedersen_hash_different_values() {
        let a = Felt::from_bytes_be_slice(&0x0807060504030201u64.to_be_bytes());
        let b = Felt::from_bytes_be_slice(&0x8070605040302010u64.to_be_bytes());

        let result = pedersen_hash(&a, &b);

        // Expected result from reference implementation
        assert_eq!(
            result.to_bytes_be(),
            hex::decode("05bbe990671c3e539518346a7513a60df1697e850540feb72f4377c061801be1")
                .unwrap()
                .as_slice()
        );
    }

    #[test]
    fn test_pedersen_array_three_elements() {
        let elements = [Felt::from(0xau64), Felt::from(0xbu64), Felt::from(0xcu64)];

        let result = pedersen_array(&elements);

        // Expected result from our implementation
        let expected =
            Felt::from_hex("0x5a9477f4c8e6d9bfb0908996294fb65a2a9224ef9696c5584fdcce1190dcb9e")
                .unwrap();

        assert_eq!(result, expected);
    }

    #[test]
    fn test_pedersen_array_empty() {
        let empty: &[Felt] = &[];
        let result = pedersen_array(empty);
        assert_eq!(result, Felt::ZERO);
    }

    #[test]
    fn test_pedersen_array_single_element() {
        let single = [Felt::from(42u64)];
        let result = pedersen_array(&single);

        // Should hash 42 with length 1
        let expected = pedersen_hash(&Felt::from(42u64), &Felt::from(1u64));
        assert_eq!(result, expected);
    }

    #[test]
    fn test_starknet_pedersen_hash_trait() {
        let data = b"hello world";
        let hash1 = StarknetPedersenHash::hash(data);
        let hash2 = StarknetPedersenHash::hash(data);

        // Same input should produce same hash
        assert_eq!(hash1, hash2);

        // Test combining two hashes using pedersen_array directly (concat_then_hash removed)
        let hashes = [hash1, hash2];
        let combined = pedersen_array(&hashes);

        // Should be deterministic
        let combined2 = pedersen_array(&hashes);
        assert_eq!(combined, combined2);
    }
}
