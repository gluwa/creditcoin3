use parity_scale_codec::{Decode, Encode};
use precompile_utils::{prelude::String, solidity::encode_arguments, solidity::Codec};
use scale_info::TypeInfo;
use sp_core::H256;
use sp_io::hashing::keccak_256;
use sp_std::cmp::Ordering;
use sp_std::vec;
use sp_std::{self, vec::Vec};

use crate::ChainId;

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec, Default)]
pub struct Query {
    pub chain_id: ChainId,
    pub height: u64,
    pub index: u64,
    pub layout_segments: Vec<LayoutSegment>,
}

impl PartialOrd for Query {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Query {
    fn cmp(&self, other: &Self) -> Ordering {
        other.height.cmp(&self.height)
    }
}

impl Query {
    pub fn id(&self) -> H256 {
        let query = self.clone();
        H256::from(keccak_256(&encode_arguments(query)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec, Copy)]
pub struct LayoutSegment {
    pub offset: u64,
    pub size: u64,
}

// note: the proof example has changed, the proof_example_erc20.json file is now
// in correspondence with the provided query and metadata
// (block 23, index 0, ERC20 Transfer data layout).
// Thus the proof is valid for this query, and should be verified against
// it successfully.
pub fn get_test_query() -> Query {
    Query {
        chain_id: 1,
        height: 23,
        index: 0,
        layout_segments: vec![
            LayoutSegment {
                offset: 448,
                size: 32,
            },
            LayoutSegment {
                offset: 192,
                size: 32,
            },
            LayoutSegment {
                offset: 224,
                size: 32,
            },
            LayoutSegment {
                offset: 800,
                size: 32,
            },
            LayoutSegment {
                offset: 928,
                size: 32,
            },
            LayoutSegment {
                offset: 960,
                size: 32,
            },
            LayoutSegment {
                offset: 992,
                size: 32,
            },
            LayoutSegment {
                offset: 1056,
                size: 32,
            },
        ],
    }
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec)]
pub struct ResultSegment {
    pub offset: u64,
    pub bytes: H256,
}

pub type ContinuityProofLength = u64;

// Exit status, result segments, and continuity proof length
pub type VerifierResponse = (
    u8,
    Vec<ResultSegment>,
    Option<ContinuityProofLength>,
    Option<H256>,
);
