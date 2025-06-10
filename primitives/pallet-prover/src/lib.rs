#![cfg_attr(not(feature = "std"), no_std)]

use attestor_primitives::ChainId;
use parity_scale_codec::{Decode, Encode};
use precompile_utils::{prelude::String, solidity::encode_arguments, solidity::Codec};
use scale_info::TypeInfo;
use sp_core::H256;
use sp_io::hashing::keccak_256;
use sp_runtime_interface::pass_by::PassByCodec;
use sp_std::vec;
use sp_std::{self, vec::Vec};

// duplicate with constants in StarkProgramMetadataStorage
// primitives/prover/src/stark_program_auth.rs, CSUB-1303
// 0xe7bdcde60ddd457ca7f344693f68f5387ed1a9de7084bfa3648d68c30266e2c4
pub const STARK_PROGRAM_V1_HASH: H256 = H256([
    231, 189, 205, 230, 13, 221, 69, 124, 167, 243, 68, 105, 63, 104, 245, 56, 126, 209, 169, 222,
    112, 132, 191, 163, 100, 141, 104, 195, 2, 102, 226, 196,
]);

// duplicate with constants in StarkProgramMetadataStorage
// primitives/prover/src/stark_program_auth.rs, CSUB-1303
// 0x173c8e8b410a5e8894dd7413f884bfeda33d20b8736c47571ad0310d002dadf9
pub const STARK_PROGRAM_V2_HASH: H256 = H256([
    23, 60, 142, 139, 65, 10, 94, 136, 148, 221, 116, 19, 248, 132, 191, 237, 163, 61, 32, 184,
    115, 108, 71, 87, 26, 208, 49, 13, 0, 45, 173, 249,
]);

// duplicate with constants in StarkProgramMetadataStorage
// primitives/prover/src/stark_program_auth.rs, CSUB-1303
// 0xa4d8a2991782c77c6e303f6090ac7afa0e8032d2473ef36df13b5aed2358a665
pub const STARK_PROGRAM_V3_HASH: H256 = H256([
    164, 216, 162, 153, 23, 130, 199, 124, 110, 48, 63, 96, 144, 172, 122, 250, 14, 128, 50, 210,
    71, 62, 243, 109, 241, 59, 90, 237, 35, 88, 166, 101,
]);

pub const U248_BYTE_COUNT: u64 = 31;

#[derive(
    Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec, Default, PassByCodec,
)]
pub struct Query {
    pub chain_id: ChainId,
    pub height: u64,
    pub index: u64,
    pub layout_segments: Vec<LayoutSegment>,
}

impl Query {
    pub fn id(&self) -> H256 {
        let query = self.clone();
        H256::from(keccak_256(&encode_arguments(query)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec)]
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
