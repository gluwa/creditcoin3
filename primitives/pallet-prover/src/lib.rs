#![cfg_attr(not(feature = "std"), no_std)]

use attestor_primitives::ChainId;
use parity_scale_codec::{Decode, Encode};
use precompile_utils::{prelude::String, solidity::encode_arguments, solidity::Codec};
use scale_info::TypeInfo;
use sp_core::H256;
use sp_io::hashing::keccak_256;
use sp_std::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq, TypeInfo, Decode, Encode, Hash, Codec, Default)]
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
}

impl Query {
    pub fn id(&self) -> H256 {
        let query = self.clone();
        H256::from(keccak_256(&encode_arguments(query)))
    }
}
