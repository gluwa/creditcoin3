use anyhow::{anyhow, Result};
use sp_core::H256;

use std::ops::Range;
use std::vec::Vec;

use pallet_prover_primitives::{LayoutSegment, Query, ResultSegment};
use utils::pedersen_hash::pedersen_array;
use utils::utils::U248_BYTE_COUNT;
use utils::Felt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeltResult {
    offset: u64,
    felts: Vec<Felt>,
}

pub fn hash_layout_segments(query: &Query) -> Result<Felt, &'static str> {
    let felt_ranges: Vec<Range<u64>> = query
        .layout_segments
        .iter()
        .map(|layout| {
            layout
                .offset
                .checked_add(layout.size)
                .map(|end| Range {
                    start: layout.offset,
                    end,
                })
                .ok_or("Overflow detected in layout segment")
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut felts_offsets = Vec::new();
    for r in felt_ranges {
        let range_len = (r.end - r.start) as usize;
        felts_offsets
            .try_reserve(range_len)
            .map_err(|_| "layout range is too large")?;
        felts_offsets.extend((r.start..r.end).map(Into::<Felt>::into));
    }

    Ok(pedersen_array(&felts_offsets))
}

pub fn get(query_felts: &[Felt], layout_segments: &[LayoutSegment]) -> Result<Vec<ResultSegment>> {
    // 1. Convert byte-based segments into felt-based offsets and sizes (31-byte alignment)
    let felt_segments = convert_segments_to_felt_segments(layout_segments);

    // 2. Sanitize felt segments
    let sanitized = sanitize(&felt_segments);

    // 3. Retrieve felt-aligned bytes from the felt array based on the felt ranges
    let result_felts = extract_felt_ranges_from_felt_array(query_felts, &sanitized);

    // 4. Convert sanitized segments into felt segments with original layout
    let felt_segments = extract_original_felt_ranges_from_sanitized(result_felts, &felt_segments);

    // 5. Convert felt-aligned extracted data back into byte-based segments
    extract_bytes_from_felts_using_original_ranges(&felt_segments, layout_segments)
}

// Bundling these to make sure they happen in the right order. Sanitizing before
// vs after converting to felts can yield different results.
pub fn convert_to_felts_then_sanitize(segments: &[LayoutSegment]) -> Vec<LayoutSegment> {
    let felt_segments = convert_segments_to_felt_segments(segments);
    sanitize(&felt_segments)
}

pub fn sanitize(segments: &[LayoutSegment]) -> Vec<LayoutSegment> {
    // Segment count already minimal
    if segments.len() <= 1 {
        return Vec::from(segments);
    }

    // Sort segments in order of least to greatest offset
    let mut sanitized = segments.to_vec();
    sanitized.sort_by(|seg_a, seg_b| seg_a.offset.cmp(&seg_b.offset));

    // Condense segments pair by pair starting from end. We start with i = sanitized.len() - 2
    // because the last pair of segment indices is (len - 2, len - 1)
    let mut i = sanitized.len() - 2;
    loop {
        let left_segment = &sanitized[i];
        let right_segment = &sanitized[i + 1];

        // Immediately adjacent counts as overlapping for our purposes. Therefore `right_segment.offset - 1`
        let overlapping = (left_segment.offset + left_segment.size - 1) >= right_segment.offset - 1;
        if overlapping {
            // Left segment offset guaranteed to be lesser due to sort
            let first_byte_index = left_segment.offset;
            let last_byte_index = (left_segment.offset + left_segment.size)
                .max(right_segment.offset + right_segment.size)
                - 1;
            let new_segment = LayoutSegment {
                offset: first_byte_index,
                size: last_byte_index - first_byte_index + 1,
            };
            // Replace two combined segments with new segment
            sanitized.remove(i + 1);
            sanitized[i] = new_segment;
        }
        // Proceed to next pair or break if this was the last pair
        if i == 0 {
            break;
        }
        i -= 1;
    }
    sanitized
}

pub fn convert_segments_to_felt_segments(segments: &[LayoutSegment]) -> Vec<LayoutSegment> {
    let mut felt_segments = Vec::with_capacity(segments.len());
    for seg in segments {
        // Inclusive last byte index of the segment
        let last_byte_index = seg.offset.saturating_add(seg.size) - 1;
        // Felt index at start and end of the segment
        let felt_offset = seg.offset / U248_BYTE_COUNT as u64;
        let felt_end = last_byte_index / U248_BYTE_COUNT as u64;
        // Number of 31-byte felts needed to cover this segment
        let felt_count = felt_end - felt_offset + 1;
        felt_segments.push(LayoutSegment {
            offset: felt_offset,
            size: felt_count,
        });
    }
    felt_segments
}

pub fn extract_felt_ranges_from_felt_array(
    felt_array: &[Felt],
    felt_ranges: &[LayoutSegment],
) -> Vec<FeltResult> {
    let mut extracted_felt_segments = Vec::with_capacity(felt_ranges.len());

    let mut felts_position: usize = 0;
    // Cairo program returns a felt_array containing only felts which are part of our ranges rather than all felts.
    for range in felt_ranges {
        let count = range.size as usize;
        let end_idx = felts_position + count;

        let segment_felts = felt_array[felts_position..end_idx].to_vec();
        let felt_result = FeltResult {
            offset: range.offset,
            felts: segment_felts,
        };
        extracted_felt_segments.push(felt_result);
        felts_position = end_idx;
    }

    extracted_felt_segments
}

fn extract_bytes_from_felts_using_original_ranges(
    felt_segments: &[FeltResult],
    original_segments: &[LayoutSegment],
) -> Result<Vec<ResultSegment>> {
    let mut segments = Vec::new();

    for (i, orig) in original_segments.iter().enumerate() {
        let mut combined_bytes = Vec::with_capacity(felt_segments[i].felts.len() * U248_BYTE_COUNT);

        for felt in &felt_segments[i].felts {
            let felt_bytes = felt.to_bytes_be(); // 32 bytes
            combined_bytes.extend_from_slice(&felt_bytes[1..]); // Skip padding byte
        }

        let start = orig.offset as usize % U248_BYTE_COUNT;
        let end = start + orig.size as usize;

        if end > combined_bytes.len() {
            return Err(anyhow!("Segment end exceeds combined bytes length"));
        }

        let relevant_bytes = &combined_bytes[start..end];

        for (j, chunk) in relevant_bytes.chunks(32).enumerate() {
            let mut padded_chunk = [0u8; 32];
            padded_chunk[..chunk.len()].copy_from_slice(chunk);
            segments.push(ResultSegment {
                offset: orig.offset + (j as u64 * 32),
                bytes: H256::from(padded_chunk),
            });
        }
    }

    Ok(segments)
}

fn extract_original_felt_ranges_from_sanitized(
    sanitized_results: Vec<FeltResult>,
    original_segments: &[LayoutSegment],
) -> Vec<FeltResult> {
    let mut result: Vec<FeltResult> = Vec::with_capacity(original_segments.len());
    for orig in original_segments {
        let mut collected_felts: Vec<Felt> = Vec::with_capacity(orig.size as usize);
        let mut collected_up_to: usize = orig.offset as usize;
        for sanitized in &sanitized_results {
            let last_sanitized_byte_idx = sanitized.offset as usize + sanitized.felts.len() - 1;
            // If part of original segment range is contained in sanitized segment, then copy bytes over
            if sanitized.offset as usize <= collected_up_to
                && last_sanitized_byte_idx >= collected_up_to
            {
                let last_orig_seg_idx = (orig.offset + orig.size - 1) as usize;
                let last_copy_idx = last_orig_seg_idx.min(last_sanitized_byte_idx);
                let sanitized_start_idx = collected_up_to - sanitized.offset as usize;
                let sanitized_end_idx = last_copy_idx - sanitized.offset as usize;

                for felt in &sanitized.felts[sanitized_start_idx..=sanitized_end_idx] {
                    collected_felts.insert(collected_up_to - orig.offset as usize, *felt);
                    collected_up_to += 1;
                }
                // All bytes collected
                if collected_up_to > last_orig_seg_idx {
                    break;
                }
            }
        }
        // When last byte of original segment is found, then add ResultSegment to result and move on
        result.push(FeltResult {
            offset: orig.offset,
            felts: collected_felts,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pallet_prover_primitives::get_test_query;

    #[test]
    fn prover_and_verifier_segment_conversions_match() {
        let query = get_test_query();
        // This process used on the prover side
        let ranges =
            prover_primitives::query::prepare_query_segments_for_prover(&query.layout_segments);
        // This process used on verifier side
        let segments = get_segments(&query);

        check_ranges_against_segments(&ranges, &segments);
    }

    #[test]
    fn segment_conversions_work_for_0_segments() {
        let mut query = get_test_query();
        query.layout_segments = vec![];

        // This process used on the prover side
        let ranges =
            prover_primitives::query::prepare_query_segments_for_prover(&query.layout_segments);
        // This process used on verifier side
        let segments = get_segments(&query);

        check_ranges_against_segments(&ranges, &segments);
    }

    #[test]
    fn get_segments_from_output_felts_works() {
        let query = get_test_query();
        let segments = get_segments(&query);
        let dummy_felts = get_dummy_felts_for_query(&query);

        let result_segments = extract_felt_ranges_from_felt_array(&dummy_felts, &segments);

        check_felt_segments_against_layout(&result_segments, &segments);
    }

    fn get_segments(query: &Query) -> Vec<LayoutSegment> {
        // Convert byte-based segments into felt-based offsets and sizes (31-byte alignment)
        let felt_segments = convert_segments_to_felt_segments(&query.layout_segments);

        // Sanitize incoming layout segments
        sanitize(&felt_segments)
    }

    fn check_ranges_against_segments(ranges: &[Range<usize>], segments: &[LayoutSegment]) {
        assert_eq!(ranges.len(), segments.len());
        for (range, segment) in ranges.iter().zip(segments) {
            assert_eq!(range.start, segment.offset as usize);
            let segment_end = segment.offset + segment.size;
            assert_eq!(range.end, segment_end as usize);
        }
    }

    fn check_felt_segments_against_layout(
        felt_segments: &[FeltResult],
        layout_segments: &[LayoutSegment],
    ) {
        assert_eq!(felt_segments.len(), layout_segments.len());
        let mut byte_value_counter: u8 = 0;
        for (result, layout) in felt_segments.iter().zip(layout_segments) {
            assert_eq!(result.felts.len(), layout.size as usize);
            for felt in &result.felts {
                let dummy_bytes = make_sample_bytes(&mut byte_value_counter);
                assert_eq!(*felt, Felt::from_bytes_be_slice(&dummy_bytes));
            }
        }
    }

    fn get_dummy_felts_for_query(query: &Query) -> Vec<Felt> {
        // Get original byte segments

        // Sum up lengths of segments and construct payload with incrementing bytes
        let felt_segments = get_segments(query);

        let sizes_sum = felt_segments
            .iter()
            .fold(0, |acc, segment| acc + segment.size as u8);

        // We expect the felts output from verify_merkle_proof.cairo to have the same number of felts
        // as the sum of the sizes of our layout segments. This property is enforced so long as
        // prover and verifier layout segment conversions match.
        let mut dummy_felts = Vec::new();
        let mut byte_value_counter = 0u8;
        for _ in 0..sizes_sum {
            let dummy_bytes = make_sample_bytes(&mut byte_value_counter);
            dummy_felts.push(Felt::from_bytes_be_slice(&dummy_bytes));
        }
        dummy_felts
    }

    fn make_sample_bytes(byte_value_counter: &mut u8) -> Vec<u8> {
        let mut dummy_bytes = Vec::new();
        // Make 31 bytes with incrementing value, so we can check byte positions in output
        for _ in 0..31 {
            dummy_bytes.push(*byte_value_counter);
            *byte_value_counter = byte_value_counter.wrapping_add(1);
        }
        dummy_bytes
    }
}
