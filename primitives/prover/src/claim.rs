use crate::claim_query::{ClaimQuery, ClaimQueryFieldError};
use crate::types::StoneProofPublicInput;
use core::cmp::max;
use core::ops::Range;
use rlp::Rlp;
use scale_info::prelude::format;
use serde::{Deserialize, Serialize};
use sp_std::{fmt, vec, vec::Vec};
use utils::block_item_traits::BlockItemIdentifier;
use utils::{
    pedersen_hash::pedersen_array,
    utils::{felts_from_bytes, felts_to_bytes, U248_BYTE_COUNT},
    Felt,
};

#[derive(Debug, PartialEq, Clone)]
pub enum ClaimValidationError {
    // proof contains not the same id that the claim
    ClaimIdNotValidated(u64, u64),
    //    ClaimIdNotValidated(ClaimIdentifier, ClaimIdentifier),
    // field at range (.0) not validated because value (.1) doesn't match expected value (.2)
    FieldNotValidated(Range<usize>, Vec<u8>, Vec<u8>),
    FieldInner(ClaimQueryFieldError),
    // query hash contained in the proof mismatches that in the claim
    // query hash contained in the proof mismatches that in the claim
    QueryOffsetsMismatch(Felt, Felt),
    // prover yielded less felts than expected
    ProofOutputTruncated,
    // prover provided a witness that given claim index exceeds the number of entities in the block
    //    ClaimOutOfBounds(ClaimOutOfBoundsWitness)
    ClaimOutOfBounds(u64),
}

impl fmt::Display for ClaimValidationError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::ClaimIdNotValidated(expected, actual) => {
                write!(
                    f,
                    "Claim ID mismatch - expected: {}, got: {}",
                    expected, actual
                )
            }
            Self::FieldNotValidated(range, expected, actual) => {
                write!(
                    f,
                    "Field validation failed at range {:?} - expected: {:?}, got: {:?}",
                    range, expected, actual
                )
            }
            Self::FieldInner(err) => write!(f, "Field inner error: {:?}", err),
            Self::QueryOffsetsMismatch(expected, actual) => {
                write!(
                    f,
                    "Query offsets mismatch - expected: {}, got: {}",
                    expected, actual
                )
            }
            Self::ProofOutputTruncated => write!(f, "Proof output was truncated"),
            Self::ClaimOutOfBounds(idx) => write!(f, "Claim index {} is out of bounds", idx),
        }
    }
}

pub type ClaimIdentifier = BlockItemIdentifier;

#[derive(Debug, Clone)]
pub struct Claim<Q: ClaimQuery> {
    pub id: ClaimIdentifier,
    pub query: Q,
    pub payload: Vec<u8>,
    //    rlp: Rlp<'a>,
    pub felt_offsets: Vec<Range<usize>>,
}

//    impl<'a, Q: ClaimQuery> Claim<'a, Q> {
impl<Q: ClaimQuery> Claim<Q> {
    pub fn try_create(
        id: ClaimIdentifier,
        query: Q,
        payload: Vec<u8>,
    ) -> Result<Self, ClaimQueryFieldError> {
        //        pub fn try_create(id: ClaimIdentifier, query: Q, rlp: Rlp<'a>) -> Result<Self, ClaimQueryFieldError> {
        let rlp = Rlp::new(&payload[..]);
        let felt_offsets = Self::felt_offsets(&query, &rlp)?;
        Ok(Self {
            id,
            query,
            payload,
            //                rlp,
            felt_offsets,
        })
    }

    pub fn id(&self) -> &ClaimIdentifier {
        &self.id
    }
    pub fn query(&self) -> &Q {
        &self.query
    }

    pub fn validate(
        &self,
        proof_public_input: &StoneProofPublicInput,
    ) -> Result<(), ClaimValidationError> {
        use core::cmp::Ordering::*;
        use ClaimValidationError::*;

        // validate claim id returned by prover
        match self.id.index().cmp(&proof_public_input.claim_index) {
            // check out-of-bounds case
            Greater => Err(ClaimOutOfBounds(proof_public_input.claim_index)),

            Equal => {
                // check if the claim falls on the first NULL leaf (out of bounds edge case)
                if felts_from_bytes(&rlp::NULL_RLP[..]) == proof_public_input.claim_fields {
                    Err(ClaimOutOfBounds(proof_public_input.claim_index))
                } else {
                    // validate query hash returned by prover
                    let local_offsets_hash = self.query_hash();
                    if proof_public_input.query_hash != local_offsets_hash {
                        return Err(QueryOffsetsMismatch(
                            proof_public_input.query_hash,
                            local_offsets_hash,
                        ));
                    }
                    // validate claim fields
                    let proof_bytes =
                        self.proof_felts_to_bytes(&proof_public_input.claim_fields)?;
                    let rlp = Rlp::new(&self.payload[..]);
                    self.query
                        .as_byte_offsets(&rlp)
                        //                        .as_byte_offsets(&self.rlp)
                        .map_err(FieldInner)?
                        .into_iter()
                        .try_for_each(|r| {
                            //                            let r_usize = r.start as usize..r.end as usize;
                            (proof_bytes[r.clone()] == rlp.as_raw()[r.clone()])
                                .then_some(())
                                .ok_or(FieldNotValidated(
                                    r.clone(),
                                    proof_bytes[r.clone()].to_vec(),
                                    rlp.as_raw()[r].to_vec(),
                                ))
                        })
                }
            }
            // claim id not validated, not out-of-bounds case
            Less => Err(ClaimIdNotValidated(
                self.id.index(),
                proof_public_input.claim_index,
            )),
        }
        // ----------- COMMENTED OUT - related to first approach compatible with MMR usage -----------
        // // try to figure out if claimer submitted an out-of-bounds claim id
        // // by checking if prover sent out-of-bounds-witness back
        // // try to extract and decode it - a single rlp-encoded u64
        // // in this case only the first felt interests
        // let proof_bytes = Self::proof_felts_to_bytes_inner(
        //     &proof_public_input.claim_fields,
        //     &[0..1],
        //     // rlp prefix byte + u64 bytes
        //     size_of::<u8>() + size_of::<u64>()
        // )?;

        // skip zero bytes
        //             let rlp_start_index = proof_bytes.iter().take_while(|&byte| byte == &0).count();

        //             if decode::<u64>(&proof_bytes[rlp_start_index..])
        //                 .map_err(|_| ClaimIdNotValidated(self.id.block_item_id.index(), proof_public_input.claim_index))?
        //                 .eq(
        //                     &proof_public_input.claim_index
        //                 ) {
        //                 // out-of-bounds-witness asserted successfully
        //                 Err(ClaimOutOfBounds(
        // //                    ClaimOutOfBoundsWitness::new(proof_public_input.claim_id.block_item_id.clone())
        //                     proof_public_input.claim_index
        //                 ))
        //             } else {
        //                 // failed to assert
        //                 Err(ClaimIdNotValidated(self.id.block_item_id.index(), proof_public_input.claim_index))
        //             }?;
    }

    /// takes query and rlp object and returns field element ranges for the prover to output and return to claimer.
    /// to prevent ambiguities these ranges must be ensured to be ordered
    /// to define ordering for ranges they are first compacted so the resulting range array
    /// contains non-overlapping ranges.
    /// for example [(3..6), (4..7), (2..4)] is compacted to [(2..7)]
    /// when ranges do not intersect the ordering for them can be defined by compare(a.start, b.start)
    fn felt_offsets(query: &Q, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        Ok(compact_and_sort_ranges(query.as_felt_offsets(rlp)?))
    }

    fn proof_felts_to_bytes(&self, proof_felts: &[Felt]) -> Result<Vec<u8>, ClaimValidationError> {
        let rlp = Rlp::new(&self.payload[..]);
        Self::proof_felts_to_bytes_inner(proof_felts, &self.felt_offsets, rlp.as_raw().len())
    }

    fn proof_felts_to_bytes_inner(
        proof_felts: &[Felt],
        felt_offsets: &[Range<usize>],
        original_bytes_len: usize,
    ) -> Result<Vec<u8>, ClaimValidationError> {
        use core::cmp::min;

        // form a buffer of original rlp length and initialize it.
        let mut bytes = vec![0u8; original_bytes_len];
        let mut i = 0;
        for r in felt_offsets {
            // byte chunk corresponding to current felt slice
            let chunk_range =
                r.start * U248_BYTE_COUNT..min(r.end * U248_BYTE_COUNT, original_bytes_len);
            let source_bytes_len = (original_bytes_len == chunk_range.end)
                .then_some(chunk_range.end - chunk_range.start);

            let chunk_end = i + r.end - r.start;
            if chunk_end > proof_felts.len() {
                return Err(ClaimValidationError::ProofOutputTruncated);
            }
            // prover outputs field elements in order determined by felt_offsets ranges on claimer side
            // prover relies on fact the claimer compacted and sorted ranges in ascending order
            let chunk = felts_to_bytes(&proof_felts[i..chunk_end], source_bytes_len);
            i += r.end - r.start;

            bytes[chunk_range].copy_from_slice(&chunk[..]);
        }
        Ok(bytes)
    }

    // TODO: make it private, now it's public for being able to compile tests
    pub fn query_hash(&self) -> Felt {
        let felts_offsets = self
            .felt_offsets
            .iter()
            .flat_map(|r| r.clone().map(Into::<Felt>::into).collect::<Vec<_>>())
            .collect::<Vec<Felt>>();
        pedersen_array(&felts_offsets[..])
    }
    // claim as seen on prover's side
}

// claim as seen on prover's side
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimSerializable {
    pub id: ClaimIdentifier,
    #[serde(deserialize_with = "deserialize_and_compact_ranges")]
    pub felt_ranges: Vec<Range<usize>>,
}

impl ClaimSerializable {
    pub fn id(&self) -> &ClaimIdentifier {
        &self.id
    }

    pub fn felt_ranges(&self) -> &[Range<usize>] {
        &self.felt_ranges
    }
}

impl<Q: ClaimQuery> From<&Claim<Q>> for ClaimSerializable {
    //    impl<Q: ClaimQuery> From<&Claim<'_, Q>> for ClaimSerializable {
    fn from(claim: &Claim<Q>) -> Self {
        Self {
            id: claim.id.clone(),
            felt_ranges: claim.felt_offsets.to_vec(),
        }
    }
}

// the claim query is expected to be sent already compacted and sorted by claimer,
// however in order to protect the prover from running Cairo program on potentially
// very large input query as a result of attack to prover's performance,
// repeat the compacting and sorting process on prover's side
fn deserialize_and_compact_ranges<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Range<usize>>, D::Error> {
    use crate::claim_query::MAX_QUERY_LEN;
    use serde::de::Error;

    let felt_offsets = Vec::<Range<usize>>::deserialize(deserializer)?;
    let len = felt_offsets.len();

    (1..MAX_QUERY_LEN)
        .contains(&len)
        .then(|| compact_and_sort_ranges(felt_offsets))
        .ok_or(D::Error::custom(format!(
            "query length is {}, expected to be in range [1..{})",
            len, MAX_QUERY_LEN
        )))
}

//impl JsonSerializable for ClaimSerializable {}
//impl JsonSerializable for Vec<ClaimSerializable> {}

fn compact_and_sort_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut compact_ranges = compact_contiguous_ranges(ranges);
    compact_ranges.sort_by(|a, b| a.start.cmp(&b.start));

    compact_ranges
}
// TODO: check worst case complexity
// TODO: could an intentionally malformed query be an attack vector to degrade claimer performance?
fn compact_contiguous_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    let mut compact_ranges = Vec::<Range<usize>>::with_capacity(ranges.len());
    let mut needs_further_compaction = false;

    for r in ranges.into_iter() {
        match compact_ranges
            .iter()
            .enumerate()
            .find_map(|(i, cr)| range_union(&r, cr).map(|r1| (i, r1)))
        {
            Some((i, r1)) => {
                needs_further_compaction = true;
                compact_ranges[i] = r1;
            }
            None => compact_ranges.push(r),
        }
    }
    if needs_further_compaction {
        compact_contiguous_ranges(compact_ranges)
    } else {
        compact_ranges
    }
}

fn range_union(r1: &Range<usize>, r2: &Range<usize>) -> Option<Range<usize>> {
    if r1.start <= r2.start && r1.end >= r2.start {
        Some(r1.start..max(r1.end, r2.end))
    } else if r2.start <= r1.start && r2.end >= r1.start {
        Some(r2.start..max(r1.end, r2.end))
    } else {
        None
    }
}
