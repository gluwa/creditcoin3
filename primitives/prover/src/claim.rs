use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use serde::{Deserialize, Serialize};
use rlp::Rlp;
use attestor_primitives::ChainId;
use crate::claim_query::{ClaimQuery, ClaimQueryFieldError};
use core::ops::Range;
use core::cmp::max;
use utils::{Felt, utils::{felts_to_bytes, U248_BYTE_COUNT}, pedersen_hash::pedersen_array};
use utils::block_item_traits::BlockItemIdentifier;

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, MaxEncodedLen, Hash)]
pub struct ClaimOld<Address> {
    pub chain_id: ChainId,
    pub block_number: u64,
    pub tx_index: u8,
    pub from: Address,
    pub to: Address,
    pub kind: ClaimKind,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    TypeInfo,
    Decode,
    Encode,
    MaxEncodedLen,
    Hash,
    Serialize,
    Deserialize,
)]
pub enum ClaimKind {
    Tx = 1,
    Rx = 2,
}

impl ClaimKind {
    pub fn subdir(&self) -> &str {
        match self {
            Self::Tx => "tx_",
            Self::Rx => "rx_",
        }
    }
}

impl Default for ClaimKind {
    fn default() -> Self {
        Self::Tx
    }
}

impl TryFrom<u8> for ClaimKind {
    type Error = u8;

    fn try_from(x: u8) -> Result<Self, u8> {
        match x {
            1 => Ok(Self::Tx),
            2 => Ok(Self::Rx),
            _ => Err(x),
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum ClaimValidationError {
    // field at range (.0) not validated because value (.1) doesn't match expected value (.2) 
    FieldNotValidated(Range<usize>, Vec<u8>, Vec<u8>),
    FieldInner(ClaimQueryFieldError),
    QueryOffsetsMismatch(Felt, Felt),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ClaimIdentifier {
    pub kind: ClaimKind,
    pub block_item_id: BlockItemIdentifier,
}

#[derive(Debug, Clone)]
pub struct Claim<'a, Q: ClaimQuery> {
    id: ClaimIdentifier,
    query: Q,
    rlp: Rlp<'a>,
    felt_offsets: Vec<Range<usize>>,
}

impl<'a, Q: ClaimQuery> Claim<'a, Q> {
    pub fn try_create(id: ClaimIdentifier, query: Q, rlp: Rlp<'a>) -> Result<Self, ClaimQueryFieldError> {
        let felt_offsets = Self::felt_offsets(&query, &rlp)?;
        Ok(
            Self {
                id,
                query,
                rlp,
                felt_offsets
            }
        )
    }

    pub fn id(&self) -> &ClaimIdentifier {
        &self.id
    }
    pub fn query(&self) -> &Q {
        &self.query
    }

    pub fn validate_fields(&self, proof_felts: &[Felt], query_hash: &Felt) -> Result<(), ClaimValidationError> {
        let local_offsets_hash = self.query_hash();
//        println!("felts_offsets: {:?}", felts_offsets.iter().map(|f| f.to_string()).collect::<Vec<_>>());
        if query_hash != &local_offsets_hash {
            return Err(ClaimValidationError::QueryOffsetsMismatch(query_hash.clone(), local_offsets_hash));
        }

        let bytes_from_proof = self.proof_felts_to_bytes(proof_felts);
        self
            .query
            .as_byte_offsets(&self.rlp)
            .map_err(ClaimValidationError::FieldInner)?
            .into_iter()
            .try_for_each(|r| 
                (bytes_from_proof[r.clone()] == self.rlp.as_raw()[r.clone()])
                    .then_some(())
                    .ok_or(ClaimValidationError::FieldNotValidated(
                        r.clone(), 
                        bytes_from_proof[r.clone()].to_vec(), 
                        self.rlp.as_raw()[r].to_vec()
                    )
                )
            )
    }

    fn felt_offsets(query: &Q, rlp: &Rlp) -> Result<Vec<Range<usize>>, ClaimQueryFieldError> {
        let mut compact_ranges = Self::compact_contiguous_ranges(query.as_felt_offsets(&rlp)?);
        compact_ranges.sort_by(|a, b| a.start.cmp(&b.start));

        Ok(compact_ranges)
    }

    fn proof_felts_to_bytes(&self, proof_felts: &[Felt]) -> Vec<u8> {
        Self::proof_felts_to_bytes_inner(proof_felts, &self.felt_offsets, self.rlp.as_raw().len())
    }

    fn proof_felts_to_bytes_inner(proof_felts: &[Felt], felt_offsets: &[Range<usize>], original_bytes_len: usize) -> Vec<u8> {
        use std::cmp::min;

        let mut bytes = vec![0u8; original_bytes_len];      
        let mut i = 0;
        for r in felt_offsets {
            let chunk_range = r.start * U248_BYTE_COUNT..min(r.end * U248_BYTE_COUNT, original_bytes_len);
            let source_bytes_len = (original_bytes_len == chunk_range.end)
                .then_some(chunk_range.end - chunk_range.start);

            let chunk = felts_to_bytes(&proof_felts[i..i + r.end - r.start], source_bytes_len);
            i += r.end - r.start;
    
            bytes[chunk_range].copy_from_slice(&chunk[..]);
        }

        bytes
    }

    // TODO: make it private. Now it is public for easier testing
    pub fn query_hash(&self) -> Felt {
        let felts_offsets = self
            .felt_offsets
            .iter()
            .map(|r| r.clone().map(Into::<Felt>::into).collect::<Vec<_>>())
            .flatten()
            .collect::<Vec<Felt>>();
        pedersen_array(&felts_offsets[..])
    }

    fn compact_contiguous_ranges(ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
        let mut compact_ranges = Vec::<Range<usize>>::new();
        let mut needs_further_compaction = false;
    
        for r in ranges.into_iter() {
            match compact_ranges
                    .iter()
                    .enumerate()
                    .find_map(|(i, cr)| 
                        Self::range_union(&r, &cr).map(|r1| (i, r1))
                    ) {
                Some((i, r1)) => {
                    needs_further_compaction = true;
                    compact_ranges[i] = r1;
                },
                None => compact_ranges.push(r),
            }
        }
        if needs_further_compaction {
            Self::compact_contiguous_ranges(compact_ranges)
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimSerializable {
    id: ClaimIdentifier,
    felt_ranges: Vec<Range<usize>>,
}

impl ClaimSerializable {
    pub fn id(&self) -> &ClaimIdentifier {
        &self.id
    }

    pub fn felt_ranges(&self) -> &[Range<usize>] {
        &self.felt_ranges
    }
}

impl<Q: ClaimQuery> From<&Claim<'_, Q>> for ClaimSerializable {
    fn from(claim: &Claim<Q>) -> Self {
        Self {
            id: claim.id.clone(),
            felt_ranges: claim.felt_offsets.to_vec(),
        }  
    }
}

// impl JsonSerializable for ClaimSerializable {}
// impl JsonSerializable for Vec<ClaimSerializable> {}

