#![cfg_attr(not(feature = "std"), no_std)]

use attestor_primitives::ChainId;
use parity_scale_codec::{Decode, Encode};
use precompile_utils::{prelude::String, solidity::encode_arguments, solidity::Codec};
use scale_info::TypeInfo;
use sp_core::H256;
use sp_io::hashing::keccak_256;
use sp_runtime_interface::pass_by::PassByCodec;
use sp_std::vec::Vec;

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

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec)]
pub struct LayoutSegment {
    pub offset: u64,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec)]
pub struct ResultSegment {
    pub offset: u64,
    pub bytes: Vec<u8>,
}

// Exit status and result segments
pub type VerifierResponse = (u8, Vec<ResultSegment>);

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash)]
pub enum VerifierExitStatus {
    // Success: proof verifies and requested byte ranges could
    // from the proof.
    Success,
    // ProofInvalid: proof verifier couldn't verify the proof.
    ProofInvalid,
    // LayoutMismatch: CCNode couldn't extract the bytes indic
    // from the submitted proof bytes. (Prover's fault)
    LayoutMismatch,
    // QueryOutOfBounds: the proof shows that either the targe
    // doesn't exist or the query's layout includes segments o
    // targeted transaction. (dApp's fault)
    QueryOutOfBounds,
    UnknownError,
}

impl Query {
    pub fn id(&self) -> H256 {
        let query = self.clone();
        H256::from(keccak_256(&encode_arguments(query)))
    }

    pub fn transform_to_felt_offsets(&mut self) {
        for segment in &mut self.layout_segments {
            segment.offset /= U248_BYTE_COUNT;
            segment.size =
                segment.size / U248_BYTE_COUNT + (segment.size % U248_BYTE_COUNT != 0) as u64;
        }
    }
}
