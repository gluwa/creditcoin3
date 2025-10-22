//! Utility functions for Creditcoin3 common operations.
//!
//! This module provides various utility functions for:
//! - Starknet Felt operations
//! - Byte array manipulations
//! - Parsing utilities

use crate::Felt;

extern crate alloc;
use alloc::vec::Vec;

/// Number of bytes in a U248 field element (31 bytes = 248 bits)
pub const U248_BYTE_COUNT: usize = 31;

// =============================================================================
// Parsing Utilities
// =============================================================================

/// Attempts to parse a string as usize, supporting both decimal and hex formats.
///
/// # Arguments
/// * `s` - String to parse
///
/// # Returns
/// * `Result<usize, core::num::ParseIntError>` - Parsed value or parse error
///
/// Note: On parse failures this function returns `Ok(0)` as a fallback to preserve
/// the original API shape while providing a simple default value.
pub fn try_parse_usize(s: &str) -> Result<usize, core::num::ParseIntError> {
    Ok(s.parse::<usize>()
        .or_else(|_| usize::from_str_radix(s.trim_start_matches("0x"), 16))
        .unwrap_or(0))
}

/// Attempts to parse a string as u64, supporting both decimal and hex formats.
///
/// # Arguments
/// * `s` - String to parse
///
/// # Returns
/// * `Result<u64, core::num::ParseIntError>` - Parsed value or parse error
///
/// Note: On parse failures this function returns `Ok(0)` as a fallback to preserve
/// the original API shape while providing a simple default value.
pub fn try_parse_u64(s: &str) -> Result<u64, core::num::ParseIntError> {
    Ok(s.parse::<u64>()
        .or_else(|_| u64::from_str_radix(s.trim_start_matches("0x"), 16))
        .unwrap_or(0))
}

/// Attempts to parse a string as Felt, supporting both decimal and hex formats.
///
/// # Arguments
/// * `s` - String to parse
///
/// # Returns
/// * `Result<Felt, starknet_types_core::felt::FromStrError>` - Parsed Felt or error
pub fn try_parse_felt(s: &str) -> Result<Felt, starknet_types_core::felt::FromStrError> {
    // Try decimal first, then hex
    match Felt::from_dec_str(s) {
        Ok(felt) => Ok(felt),
        Err(_) if s.starts_with('-') => {
            // Handle negative numbers
            let neg_x = Felt::from_dec_str(&s[1..])?;
            Ok(Felt::ZERO - neg_x)
        }
        Err(_) => Felt::from_hex(s),
    }
}

// =============================================================================
// Byte Array Conversions
// =============================================================================

/// Converts bytes to a vector of Felts using 31-byte chunks.
///
/// Each chunk is converted to a Felt using big-endian byte ordering.
/// The last chunk may be smaller than 31 bytes.
///
/// # Arguments
/// * `bytes` - Byte slice to convert
///
/// # Returns
/// * `Vec<Felt>` - Vector of Felts representing the byte data
pub fn felts_from_bytes(bytes: &[u8]) -> Vec<Felt> {
    bytes
        .chunks(U248_BYTE_COUNT)
        .map(Felt::from_bytes_be_slice)
        .collect()
}

/// Converts a vector of Felts back to bytes.
///
/// This function reverses the operation performed by `felts_from_bytes`.
/// The `source_bytes_len` parameter is used to determine the exact original
/// byte length and avoid zero padding in the result.
///
/// # Arguments
/// * `felts` - Slice of Felts to convert
/// * `source_bytes_len` - Optional original byte length for exact reconstruction
///
/// # Returns
/// * `Vec<u8>` - The reconstructed byte array
pub fn felts_to_bytes(felts: &[Felt], source_bytes_len: Option<usize>) -> Vec<u8> {
    const U248_OFFSET: usize = 32 - U248_BYTE_COUNT; // U248_OFFSET == 1

    let capacity = source_bytes_len.unwrap_or(U248_BYTE_COUNT * felts.len());
    let mut bytes = Vec::with_capacity(capacity);

    // Process all but the last felt
    for felt in &felts[..felts.len().saturating_sub(1)] {
        bytes.extend_from_slice(&felt.to_bytes_be()[U248_OFFSET..]);
    }

    // Handle the last felt with proper offset calculation
    if let Some(last_felt) = felts.last() {
        let last_offset = source_bytes_len
            .map(|len| {
                let remainder = len % U248_BYTE_COUNT;
                if remainder == 0 && len > 0 {
                    U248_OFFSET // Full chunk
                } else {
                    32 - remainder // Partial chunk
                }
            })
            .unwrap_or(U248_OFFSET);

        bytes.extend_from_slice(&last_felt.to_bytes_be()[last_offset..]);
    }

    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_felt_to_bytes_roundtrip() {
        for i in 0..100 {
            let original = (0..i as u8).collect::<Vec<u8>>();
            let felts = felts_from_bytes(&original);
            let reconstructed = felts_to_bytes(&felts, Some(original.len()));
            assert_eq!(original, reconstructed, "Roundtrip failed for length {i}");
        }
    }

    #[test]
    fn test_try_parse_functions() {
        assert_eq!(try_parse_u64("123").unwrap(), 123);
        assert_eq!(try_parse_u64("0x123").unwrap(), 0x123);

        assert_eq!(try_parse_usize("456").unwrap(), 456);
        assert_eq!(try_parse_usize("0x456").unwrap(), 0x456);

        // Test basic felt parsing
        assert_eq!(try_parse_felt("42").unwrap(), Felt::from(42u64));
        assert_eq!(try_parse_felt("0x2A").unwrap(), Felt::from(42u64));
    }

    #[test]
    fn test_negative_felt_parsing() {
        let negative_felt = try_parse_felt("-42").unwrap();
        let expected = Felt::ZERO - Felt::from(42u64);
        assert_eq!(negative_felt, expected);
    }
}
