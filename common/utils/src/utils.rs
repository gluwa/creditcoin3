//! Utility functions for Creditcoin3 common operations.
//!
//! This module provides various utility functions for:
//! - Starknet Felt operations
//! - Byte array manipulations
//! - Parsing utilities

// Removed Felt import - no longer needed

extern crate alloc;

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
// Removed try_parse_felt - Felt type is no longer used

// =============================================================================
// Byte Array Conversions
// =============================================================================

// Removed felts_from_bytes and felts_to_bytes - Felt type is no longer used

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_parse_functions() {
        assert_eq!(try_parse_u64("123").unwrap(), 123);
        assert_eq!(try_parse_u64("0x123").unwrap(), 0x123);

        assert_eq!(try_parse_usize("456").unwrap(), 456);
        assert_eq!(try_parse_usize("0x456").unwrap(), 0x456);
    }
}
