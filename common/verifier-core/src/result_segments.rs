use anyhow::{anyhow, Result};
use sp_core::H256;
use std::vec::Vec;

use pallet_prover_primitives::{LayoutSegment, Query, ResultSegment};
use utils::pedersen_hash::pedersen_array;
use utils::utils::U248_BYTE_COUNT;
use utils::Felt; // Re-exported from starknet_crypto::Felt (see common/utils/src/lib.rs)

/// Computes the Pedersen hash of felt indices covered by the layout segments.
///
/// IMPORTANT: This function expects `query.layout_segments` to already be converted
/// to felt-based segments (not byte-based). The segments are expanded into individual
/// felt indices and then hashed.
///
/// Example:
///   Input: LayoutSegment{offset: 6, size: 2} (felt-based)
///   Expands to: [6, 7]
///   Hashes: pedersen([6, 7, 2]) where 2 is the array length
///
/// For byte-based segments, first call `convert_byte_segments_to_felt_segments()`.
pub fn hash_felt_indices(query: &Query) -> Result<Felt, &'static str> {
    let mut felt_indices = Vec::new();

    for segment in &query.layout_segments {
        let end = segment
            .offset
            .checked_add(segment.size)
            .ok_or("Overflow in felt segment")?;

        // Expand felt range into individual indices
        // e.g., offset=6, size=2 → [6, 7]
        felt_indices.extend((segment.offset..end).map(Felt::from));
    }

    Ok(pedersen_array(&felt_indices))
}

/// Extracts byte segments from felt array using pre-merged felt segments.
///
/// This is an optimization for when the caller has already computed the merged
/// felt segments (e.g., during query validation).
///
/// Pipeline:
/// 1. Extract bytes from the felt array (using pre-merged segments)
/// 2. Map back to original byte segment boundaries
///
/// Example usage:
///   During validation, we compute merged_felt_segments to hash them.
///   Later, when extracting result segments, we reuse those same segments
///   instead of recomputing the byte->felt conversion and merge.
pub fn get_with_merged_segments(
    query_felts: &[Felt],
    merged_felt_segments: &[LayoutSegment],
    byte_segments: &[LayoutSegment],
) -> Result<Vec<ResultSegment>> {
    if byte_segments.is_empty() {
        return Ok(Vec::new());
    }

    // Extract raw bytes from the felt array using the pre-merged segments
    let felt_bytes = extract_bytes_from_felts(query_felts, merged_felt_segments)?;

    // Map extracted bytes back to original byte segment boundaries
    extract_result_segments(&felt_bytes, byte_segments)
}

/// Converts byte-based segments to felt-based segments, then merges overlapping ones.
///
/// This is the conversion used by both prover and verifier to ensure they agree
/// on which felts to read. The result is used to compute the query hash.
///
/// Example:
///   Input: [LayoutSegment{offset: 192, size: 32}, LayoutSegment{offset: 200, size: 10}]
///   Step 1: [{offset: 6, size: 2}, {offset: 6, size: 1}] (both span felt 6)
///   Step 2: [{offset: 6, size: 2}] (merged)
pub fn convert_byte_segments_to_felt_segments_and_merge(
    byte_segments: &[LayoutSegment],
) -> Vec<LayoutSegment> {
    let felt_segments = convert_byte_segments_to_felt_segments(byte_segments);
    merge_overlapping_segments(&felt_segments)
}

/// Converts byte-based segments to felt-based segments (31-byte alignment).
///
/// ## Why Felts?
///
/// Cairo/STARK proofs operate on **field elements (felts)**, not raw bytes. Each felt
/// is 248 bits (31 bytes) of usable data. The transaction data in the Merkle tree is
/// pre-converted to felts before storage, so Cairo must work with felt indices.
///
/// **Type Definition:**
/// - `Felt` = `starknet_crypto::Felt` (re-exported via `common/utils/src/lib.rs`)
/// - Crate: <https://docs.rs/starknet-crypto/latest/starknet_crypto/struct.Felt.html>
///
/// **References:**
/// - See `docs/architecture/WHY_FELTS_NOT_BYTES.md` for detailed explanation
/// - Cairo program: `cairo/scripts/verify_merkle_proof.cairo`
/// - Starknet docs: <https://docs.starknet.io/documentation/architecture_and_concepts/Smart_Contracts/cairo-and-sierra/>
/// - Felt spec: <https://docs.starknet.io/documentation/architecture_and_concepts/Cryptography/p-value/>
///
/// ## Felt Encoding
///
/// Each felt holds 31 bytes (U248). This function calculates which felts
/// are needed to cover the requested byte ranges.
///
/// ### Encoding Details:
/// - **Felt size**: 32 bytes total (252 bits field)
/// - **Usable data**: 31 bytes (248 bits) - first byte is padding/zero
/// - **Encoding**: `felt.to_bytes_be()[1..]` extracts the 31 usable bytes
/// - **Cairo storage**: Transaction data stored as `felt[]` in Merkle tree
/// - **Constant**: `U248_BYTE_COUNT = 31` (defined in `common/utils/src/utils.rs`)
///
/// ### Mapping Formula:
/// ```
/// felt_index = byte_offset ÷ 31
/// ```
///
/// Example:
///   Felt 0 = bytes [0..31)
///   Felt 1 = bytes [31..62)
///   Felt 6 = bytes [186..217)
///   Felt 7 = bytes [217..248)
///
///   Input: LayoutSegment{offset: 192, size: 32} (bytes 192-223)
///   Output: LayoutSegment{offset: 6, size: 2} (felts 6-7)
///
/// ### Implementation:
/// ```
/// first_felt = byte_offset ÷ 31
/// last_felt = (byte_offset + size - 1) ÷ 31
/// felt_count = last_felt - first_felt + 1
/// ```
fn convert_byte_segments_to_felt_segments(byte_segments: &[LayoutSegment]) -> Vec<LayoutSegment> {
    byte_segments
        .iter()
        .map(|segment| {
            // Which felt contains the first byte?
            let first_felt_index = segment.offset / U248_BYTE_COUNT as u64;

            // Which felt contains the last byte?
            let last_byte_index = segment.offset + segment.size - 1;
            let last_felt_index = last_byte_index / U248_BYTE_COUNT as u64;

            // How many felts do we need?
            let felt_count = last_felt_index - first_felt_index + 1;

            LayoutSegment {
                offset: first_felt_index,
                size: felt_count,
            }
        })
        .collect()
}

/// Merges overlapping or adjacent segments into minimal set.
///
/// This optimization ensures Cairo only reads each felt once, even if
/// multiple byte segments overlap the same felt.
///
/// Example:
///   Input: [{offset: 5, size: 2}, {offset: 6, size: 2}]
///   Overlaps at felt 6
///   Output: [{offset: 5, size: 3}] (covers felts 5, 6, 7)
fn merge_overlapping_segments(segments: &[LayoutSegment]) -> Vec<LayoutSegment> {
    if segments.is_empty() {
        return Vec::new();
    }

    // Sort by offset
    let mut sorted = segments.to_vec();
    sorted.sort_by_key(|s| s.offset);

    let mut merged = Vec::new();
    let mut current = sorted[0];

    for next in sorted.iter().skip(1) {
        let current_end = current.offset + current.size;

        // Segments overlap or are adjacent if next starts at or before current ends
        if next.offset <= current_end {
            // Merge: extend current to cover both
            let next_end = next.offset + next.size;
            current.size = next_end.max(current_end) - current.offset;
        } else {
            // Gap between segments: save current and start new
            merged.push(current);
            current = *next;
        }
    }
    merged.push(current);

    merged
}

/// Extracts bytes from felt array based on felt-aligned segments.
///
/// The Cairo program outputs felts in sequential order corresponding to
/// the merged felt segments. Each felt is 32 bytes but only the last 31
/// bytes are used (first byte is padding).
///
/// ## Felt to Bytes Conversion
///
/// Each felt in the Cairo output is decoded as:
/// ```rust
/// let felt_bytes = felt.to_bytes_be();        // 32 bytes (big-endian)
/// let usable_bytes = &felt_bytes[1..];        // 31 bytes (skip padding)
/// ```
///
/// ## Practical Example
///
/// ```rust
/// // Given: Transaction data with 100 bytes
/// // Want: Bytes 65-95 (31 bytes)
///
/// // Step 1: Which felts contain this data?
/// // Felt[2] covers bytes 62-92 (31 bytes)
/// // Felt[3] covers bytes 93-123 (31 bytes)
/// // So we need felts 2 and 3 (62 bytes total)
///
/// // Step 2: Cairo outputs these felts
/// let query_felts = vec![felt2, felt3];
///
/// // Step 3: Extract bytes from felts
/// let felt2_bytes = felt2.to_bytes_be()[1..]; // 31 bytes [62..93)
/// let felt3_bytes = felt3.to_bytes_be()[1..]; // 31 bytes [93..124)
///
/// // Step 4: Extract our requested range [65..96)
/// // From felt2_bytes: skip 3 bytes (65-62=3), take 28 bytes
/// // From felt3_bytes: take 3 bytes (to complete 31 bytes)
/// ```
///
/// **Reference implementations:**
/// - **Prover**: `primitives/prover/src/query.rs` → `byte_segments_into_felt_ranges()`
/// - **Verifier**: This function (extraction from felts)
/// - **Cairo**: `cairo/scripts/verify_merkle_proof.cairo`
/// - **Documentation**: `docs/architecture/WHY_FELTS_NOT_BYTES.md`
///
/// Returns: Vector of (felt_offset, bytes) tuples
fn extract_bytes_from_felts(
    query_felts: &[Felt],
    felt_segments: &[LayoutSegment],
) -> Result<Vec<(u64, Vec<u8>)>> {
    let mut result = Vec::new();
    let mut felt_index = 0;

    for segment in felt_segments {
        let num_felts = segment.size as usize;
        let end_index = felt_index + num_felts;

        if end_index > query_felts.len() {
            return Err(anyhow!(
                "Not enough felts in Cairo output: expected {}, got {}",
                end_index,
                query_felts.len()
            ));
        }

        // Convert felts to bytes
        let mut bytes = Vec::with_capacity(num_felts * U248_BYTE_COUNT);
        for felt in &query_felts[felt_index..end_index] {
            let felt_bytes = felt.to_bytes_be(); // 32 bytes
            bytes.extend_from_slice(&felt_bytes[1..]); // Skip first padding byte, use 31 bytes
        }

        result.push((segment.offset, bytes));
        felt_index = end_index;
    }

    Ok(result)
}

/// Maps felt-aligned bytes back to original byte segment boundaries.
///
/// The felt-aligned bytes may contain more data than requested (since felts
/// are 31 bytes). This function extracts only the exact byte ranges specified
/// in the original segments and splits them into 32-byte chunks for ResultSegment.
fn extract_result_segments(
    felt_bytes: &[(u64, Vec<u8>)],
    byte_segments: &[LayoutSegment],
) -> Result<Vec<ResultSegment>> {
    let mut results = Vec::new();

    for segment in byte_segments {
        let segment_start = segment.offset;
        let segment_end = segment.offset + segment.size;

        // Collect bytes for this segment from the felt-aligned data
        let mut segment_bytes = Vec::new();

        for (felt_offset, bytes) in felt_bytes {
            let felt_start_byte = felt_offset * U248_BYTE_COUNT as u64;
            let felt_end_byte = felt_start_byte + bytes.len() as u64;

            // Check if this felt-aligned chunk overlaps with our segment
            if felt_start_byte < segment_end && felt_end_byte > segment_start {
                // Calculate overlap range
                let copy_start = segment_start.max(felt_start_byte);
                let copy_end = segment_end.min(felt_end_byte);

                // Calculate indices into the felt's byte array
                let local_start = (copy_start - felt_start_byte) as usize;
                let local_end = (copy_end - felt_start_byte) as usize;

                segment_bytes.extend_from_slice(&bytes[local_start..local_end]);
            }
        }

        // Verify we got the right amount of data
        if segment_bytes.len() != segment.size as usize {
            return Err(anyhow!(
                "Extracted {} bytes but expected {} for segment at offset {}",
                segment_bytes.len(),
                segment.size,
                segment.offset
            ));
        }

        // Split into 32-byte chunks (ResultSegment requirement)
        for (chunk_index, chunk) in segment_bytes.chunks(32).enumerate() {
            let mut padded = [0u8; 32];
            padded[..chunk.len()].copy_from_slice(chunk);

            results.push(ResultSegment {
                offset: segment.offset + (chunk_index as u64 * 32),
                bytes: H256::from(padded),
            });
        }
    }

    Ok(results)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use pallet_prover_primitives::get_test_query;

    #[test]
    fn test_byte_to_felt_conversion() {
        // Byte range [0..31) = Felt 0 only
        let segment = LayoutSegment {
            offset: 0,
            size: 31,
        };
        let result = convert_byte_segments_to_felt_segments(&[segment]);
        assert_eq!(result[0].offset, 0);
        assert_eq!(result[0].size, 1);

        // Byte range [0..32) = Felts 0 and 1
        let segment = LayoutSegment {
            offset: 0,
            size: 32,
        };
        let result = convert_byte_segments_to_felt_segments(&[segment]);
        assert_eq!(result[0].offset, 0);
        assert_eq!(result[0].size, 2);

        // Byte range [31..63) = Felts 1 and 2
        let segment = LayoutSegment {
            offset: 31,
            size: 32,
        };
        let result = convert_byte_segments_to_felt_segments(&[segment]);
        assert_eq!(result[0].offset, 1);
        assert_eq!(result[0].size, 2);

        // Byte range [192..224) = Felts 6 and 7 (since 192÷31=6.19...)
        let segment = LayoutSegment {
            offset: 192,
            size: 32,
        };
        let result = convert_byte_segments_to_felt_segments(&[segment]);
        assert_eq!(result[0].offset, 6);
        assert_eq!(result[0].size, 2);
    }

    #[test]
    fn test_merge_empty() {
        assert_eq!(merge_overlapping_segments(&[]), vec![]);
    }

    #[test]
    fn test_merge_single() {
        let seg = LayoutSegment {
            offset: 5,
            size: 10,
        };
        assert_eq!(merge_overlapping_segments(&[seg]), vec![seg]);
    }

    #[test]
    fn test_merge_overlapping() {
        let segments = vec![
            LayoutSegment { offset: 0, size: 5 },
            LayoutSegment { offset: 3, size: 5 }, // Overlaps at 3-4
        ];
        let result = merge_overlapping_segments(&segments);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].offset, 0);
        assert_eq!(result[0].size, 8); // Covers 0-7
    }

    #[test]
    fn test_merge_adjacent() {
        let segments = vec![
            LayoutSegment { offset: 0, size: 5 },
            LayoutSegment { offset: 5, size: 5 }, // Adjacent
        ];
        let result = merge_overlapping_segments(&segments);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].offset, 0);
        assert_eq!(result[0].size, 10);
    }

    #[test]
    fn test_merge_separate() {
        let segments = vec![
            LayoutSegment { offset: 0, size: 5 },
            LayoutSegment {
                offset: 10,
                size: 5,
            }, // Gap
        ];
        let result = merge_overlapping_segments(&segments);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_merge_unsorted() {
        let segments = vec![
            LayoutSegment {
                offset: 10,
                size: 5,
            },
            LayoutSegment { offset: 0, size: 5 },
            LayoutSegment { offset: 4, size: 8 }, // Overlaps both
        ];
        let result = merge_overlapping_segments(&segments);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].offset, 0);
        assert_eq!(result[0].size, 15);
    }

    #[test]
    fn test_hash_felt_indices() {
        // Create felt-based segments (not byte-based!)
        let felt_segments = vec![
            LayoutSegment { offset: 6, size: 2 }, // Felts 6-7
            LayoutSegment {
                offset: 10,
                size: 1,
            }, // Felt 10
        ];
        let query = Query {
            chain_id: 1,
            height: 1,
            index: 0,
            layout_segments: felt_segments,
        };

        // This should hash [6, 7, 10] (3 felt indices)
        let result = hash_felt_indices(&query);
        assert!(result.is_ok());
    }

    #[test]
    fn test_prover_verifier_conversion_match() {
        let query = get_test_query();

        // Prover side
        let prover_ranges =
            prover_primitives::query::prepare_query_segments_for_prover(&query.layout_segments);

        // Verifier side
        let verifier_segments =
            convert_byte_segments_to_felt_segments_and_merge(&query.layout_segments);

        // Should produce identical results
        assert_eq!(prover_ranges.len(), verifier_segments.len());
        for (range, segment) in prover_ranges.iter().zip(&verifier_segments) {
            assert_eq!(range.start, segment.offset as usize);
            assert_eq!(range.end, (segment.offset + segment.size) as usize);
        }
    }

    #[test]
    fn test_empty_segments() {
        let segments: Vec<LayoutSegment> = vec![];
        let prover_ranges = prover_primitives::query::prepare_query_segments_for_prover(&segments);
        let verifier_segments = convert_byte_segments_to_felt_segments_and_merge(&segments);

        assert_eq!(prover_ranges.len(), 0);
        assert_eq!(verifier_segments.len(), 0);
    }

    #[test]
    fn test_extract_bytes_from_felts() {
        // Create test felts
        let felt1 = Felt::from(1u64);
        let felt2 = Felt::from(2u64);
        let felts = vec![felt1, felt2];

        let segments = vec![LayoutSegment { offset: 0, size: 2 }];

        let result = extract_bytes_from_felts(&felts, &segments).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 0); // offset
        assert_eq!(result[0].1.len(), 62); // 2 felts × 31 bytes
    }

    #[test]
    fn test_overflow_protection() {
        let segment = LayoutSegment {
            offset: u64::MAX,
            size: 1,
        };
        let query = Query {
            chain_id: 1,
            height: 1,
            index: 0,
            layout_segments: vec![segment],
        };

        let result = hash_felt_indices(&query);
        assert!(result.is_err());
    }
}
