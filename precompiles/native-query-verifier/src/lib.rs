#![cfg_attr(not(feature = "std"), no_std)]

//! Native Query Verifier Precompile
//!
//! This precompile provides native-speed verification of blockchain queries using:
//! - Merkle proof verification for transaction inclusion
//! - Continuity chain validation for block attestations
//! - Data extraction from verified transactions
//!
//! The precompile is accessible at address 0x0FD2 (4050 in decimal)

use attestor_primitives::{block::Block, query::Query};
use core::marker::PhantomData;
use ethabi::{encode, Token};
use fp_evm::{ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::error;
use pallet_evm::AddressMapping;
use precompile_utils::{evm::logs::log2, keccak256, prelude::*, solidity::Codec};
use sp_core::H256;
use sp_std::vec;
use sp_std::vec::Vec;

// Use the QueryMerkleProof from the mmr crate
use mmr::query_proof::QueryMerkleProof;

// Type alias for compatibility
type MerkleProof = QueryMerkleProof;

// Event selectors (keccak256 of event signatures)
/// QueryVerified(address,bytes32,uint64,uint64,uint8,(uint64,bytes32)[])
pub const SELECTOR_LOG_QUERY_VERIFIED: [u8; 32] =
    keccak256!("QueryVerified(address,bytes32,uint64,uint64,uint8,(uint64,bytes32)[])");
/// BatchQueriesVerified(uint256,uint256,uint256)
pub const SELECTOR_LOG_BATCH_QUERIES_VERIFIED: [u8; 32] =
    keccak256!("BatchQueriesVerified(uint256,uint256,uint256)");

#[cfg(test)]
mod mock;
#[cfg(test)]
mod test_helpers;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_full_coverage;
#[cfg(test)]
mod tests_gas_security;
#[cfg(test)]
mod tests_view;

mod verify;
use verify::ResultSegment;
mod continuity;

/// Native Query Verifier Precompile
///
/// Provides efficient, native-speed verification of blockchain queries by validating:
/// 1. Merkle proofs for transaction inclusion in blocks
/// 2. Continuity chains linking blocks to attested checkpoints
/// 3. Data extraction from verified transactions
///
/// This precompile enables trustless cross-chain data verification without requiring
/// the full blockchain state, making it ideal for bridges and oracles.
pub struct NativeQueryVerifierPrecompile<Runtime>(PhantomData<Runtime>);

// Size constraints
type ConstU10MB = sp_core::ConstU32<10_485_760>; // Type alias for bounded vec

// Batch queries constraint
type MaxBatchSize = sp_core::ConstU32<10>;

/// Result of query verification
///
/// Contains the verification status and any extracted data segments.
/// Note: In the current implementation, functions revert on failure rather than
/// returning error status codes, so status will always be 0 (Success) if returned.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct QueryVerificationResult {
    /// Verification status (always 0 = Success in current implementation)
    /// Legacy status codes: 1 = MerkleProofInvalid, 2 = ContinuityChainInvalid,
    /// 3 = DataExtractionError, 4 = MerkleRootMismatch
    pub status: u8,
    /// Extracted data segments from the verified transaction
    pub result_segments: Vec<ResultSegment>,
}

/// Result of batch query verification
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct BatchQueryVerificationResult {
    /// Number of successfully verified queries
    pub successful_queries: u32,
    /// Number of failed queries
    pub failed_queries: u32,
    /// Individual results for each query
    pub results: Vec<QueryVerificationResult>,
}

/// Helper function to encode revert messages in Solidity format
pub fn encode_revert_message(message: &str) -> Vec<u8> {
    // Function selector for Error(string)
    let mut revert_with_selector = [0x08, 0xc3, 0x79, 0xa0].to_vec();
    let encoded_revert = encode(&[Token::String(message.into())]);
    revert_with_selector.extend(encoded_revert);
    revert_with_selector
}

#[precompile_utils::precompile]
impl<Runtime> NativeQueryVerifierPrecompile<Runtime>
where
    Runtime: pallet_evm::Config + frame_system::Config + pallet_attestation_poc::Config,
    Runtime::Hash: Into<H256>,
    H256: Into<Runtime::Hash>,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    /// Verify a blockchain query with Merkle proof and continuity chain (view function)
    ///
    /// This is a read-only version that doesn't emit events. It charges the same gas
    /// as the non-view function but doesn't modify state or emit logs.
    /// Useful for off-chain verification or when events are not needed.
    ///
    /// # Parameters
    /// - `query`: The query specification (chain_id, block height, data layout segments)
    /// - `tx_data`: The raw transaction data to verify
    /// - `merkle_proof`: Merkle proof for transaction inclusion in the block
    /// - `continuity_blocks`: Chain of blocks from queryHeight-1 to next attestation/checkpoint
    ///
    /// # Returns
    /// `QueryVerificationResult` with status (0) and extracted data segments
    ///
    /// # Reverts
    /// - If continuity chain is empty or invalid
    /// - If merkle proof verification fails
    /// - If merkle root doesn't match continuity block
    /// - If query block not found in continuity chain
    /// - If continuity chain doesn't end at a valid attestation/checkpoint
    #[precompile::public("verifyQueryView((uint64,uint64,(uint64,uint64)[]),bytes,(bytes32,(bytes32,bool)[]),(uint64,bytes32,bytes32,bytes32)[])")]
    #[precompile::view]
    fn verify_query_view(
        handle: &mut impl PrecompileHandle,
        query: Query,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<Block>,
    ) -> EvmResult<QueryVerificationResult> {
        Self::verify_query_impl(
            handle,
            query,
            tx_data,
            merkle_proof,
            continuity_blocks,
            false, // don't emit events (view function)
        )
    }

    /// Verify a blockchain query with Merkle proof and continuity chain
    ///
    /// This is the state-changing version that emits a `QueryVerified` event on success.
    /// The event contains the query ID, chain ID, height, status, and extracted segments.
    ///
    /// # Parameters
    /// - `query`: The query specification (chain_id, block height, data layout segments)
    /// - `tx_data`: The raw transaction data to verify
    /// - `merkle_proof`: Merkle proof for transaction inclusion in the block
    /// - `continuity_blocks`: Chain of blocks from queryHeight-1 to next attestation/checkpoint
    ///
    /// # Returns
    /// `QueryVerificationResult` with status (0) and extracted data segments
    ///
    /// # Events
    /// Emits `QueryVerified(address,bytes32,uint64,uint64,uint8,(uint64,bytes32)[])`
    ///
    /// # Reverts
    /// - If continuity chain is empty or invalid
    /// - If merkle proof verification fails
    /// - If merkle root doesn't match continuity block
    /// - If query block not found in continuity chain
    /// - If continuity chain doesn't end at a valid attestation/checkpoint
    #[precompile::public("verifyQuery((uint64,uint64,(uint64,uint64)[]),bytes,(bytes32,(bytes32,bool)[]),(uint64,bytes32,bytes32,bytes32)[])")]
    fn verify_query(
        handle: &mut impl PrecompileHandle,
        query: Query,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<Block>,
    ) -> EvmResult<QueryVerificationResult> {
        Self::verify_query_impl(
            handle,
            query,
            tx_data,
            merkle_proof,
            continuity_blocks,
            true, // emit events (non-view function)
        )
    }

    /// Verify a batch of queries with shared continuity proof (view function)
    ///
    /// This is a read-only version that doesn't emit events. Optimized for batch
    /// verification by validating the continuity chain once and reusing it for all queries.
    /// This can save ~40% gas compared to individual verifications.
    ///
    /// # Arguments
    /// * `queries` - Vector of queries to verify (max 10)
    /// * `tx_data_array` - Transaction data for each query (must match queries length)
    /// * `merkle_proofs` - Merkle proofs for each query (must match queries length)
    /// * `shared_continuity_blocks` - Shared continuity chain covering all query heights
    ///
    /// # Returns
    /// `BatchQueryVerificationResult` with success/failure counts and individual results
    ///
    /// # Reverts
    /// - If input arrays have mismatched lengths
    /// - If shared continuity chain is invalid
    #[precompile::public("verifyBatchQueriesView((uint64,uint64,(uint64,uint64)[])[],bytes[],(bytes32,(bytes32,bool)[])[],(uint64,bytes32,bytes32,bytes32)[])")]
    #[precompile::view]
    fn verify_batch_queries_view(
        handle: &mut impl PrecompileHandle,
        queries: BoundedVec<Query, MaxBatchSize>,
        tx_data_array: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<MerkleProof>,
        shared_continuity_blocks: Vec<Block>,
    ) -> EvmResult<BatchQueryVerificationResult> {
        Self::verify_batch_queries_impl(
            handle,
            queries,
            tx_data_array,
            merkle_proofs,
            shared_continuity_blocks,
            false, // don't emit events (view function)
        )
    }

    /// Verify a batch of queries with shared continuity proof
    ///
    /// This is the state-changing version that emits a `BatchQueriesVerified` event.
    /// Optimized for batch verification:
    /// 1. Validates the continuity chain once (shared across all queries)
    /// 2. For each query, verifies:
    ///    - Merkle proof for transaction inclusion
    ///    - Query block exists in continuity chain
    ///    - Merkle root matches the continuity block
    ///    - Data extraction from transaction
    ///
    /// # Arguments
    /// * `queries` - Vector of queries to verify (max 10)
    /// * `tx_data_array` - Transaction data for each query (must match queries length)
    /// * `merkle_proofs` - Merkle proofs for each query (must match queries length)
    /// * `shared_continuity_blocks` - Shared continuity chain covering all query heights
    ///
    /// # Returns
    /// `BatchQueryVerificationResult` with success/failure counts and individual results
    ///
    /// # Events
    /// Emits `BatchQueriesVerified(uint256,uint256,uint256)` with total queries,
    /// successful queries, and failed queries counts
    ///
    /// # Reverts
    /// - If input arrays have mismatched lengths
    /// - If shared continuity chain is invalid
    #[precompile::public("verifyBatchQueries((uint64,uint64,(uint64,uint64)[])[],bytes[],(bytes32,(bytes32,bool)[])[],(uint64,bytes32,bytes32,bytes32)[])")]
    fn verify_batch_queries(
        handle: &mut impl PrecompileHandle,
        queries: BoundedVec<Query, MaxBatchSize>,
        tx_data_array: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<MerkleProof>,
        shared_continuity_blocks: Vec<Block>,
    ) -> EvmResult<BatchQueryVerificationResult> {
        Self::verify_batch_queries_impl(
            handle,
            queries,
            tx_data_array,
            merkle_proofs,
            shared_continuity_blocks,
            true, // emit events (non-view function)
        )
    }

    /// Generic handler for verification failures
    ///
    /// Logs the error and returns a revert with a descriptive message.
    /// In the current implementation, all failures revert rather than
    /// returning error status codes, making verification results binary
    /// (success with data or revert with error message).
    fn handle_verification_failure(
        _handle: &mut impl PrecompileHandle,
        query: &Query,
        _status: u8,
        message: &str,
    ) -> EvmResult<QueryVerificationResult> {
        error!("{} for query: {:?}", message, query.id());

        // Revert with the error message instead of emitting an event
        Err(PrecompileFailure::Revert {
            output: encode_revert_message(message),
            exit_status: ExitRevert::Reverted,
        })
    }

    /// Emit success event for verified query
    ///
    /// Emits a QueryVerified event with the query details and extracted data segments.
    /// This event allows off-chain systems to track successful verifications.
    fn emit_verification_success(
        handle: &mut impl PrecompileHandle,
        query: &Query,
        result_segments: &[ResultSegment],
    ) -> Result<(), PrecompileFailure> {
        let result_tokens: Vec<Token> = result_segments
            .iter()
            .map(|segment| {
                Token::Tuple(vec![
                    Token::Uint(segment.offset.into()),
                    Token::FixedBytes(segment.bytes.0.to_vec()),
                ])
            })
            .collect();

        // Emit the success event directly
        let event_data = ethabi::encode(&[
            Token::FixedBytes(query.id().0.to_vec()),
            Token::Uint(query.chain_id.into()),
            Token::Uint(query.height.into()),
            Token::Uint(0u8.into()), // Success status
            Token::Array(result_tokens),
        ]);

        log2(
            handle.context().address,
            SELECTOR_LOG_QUERY_VERIFIED,
            handle.context().caller,
            event_data,
        )
        .record(handle)?;

        Ok(())
    }

    /// Get an attestation from storage by chain key and digest
    ///
    /// Retrieves a signed attestation that anchors a specific block digest
    /// to the Creditcoin3 consensus. Used to validate continuity chain endpoints.
    fn get_attestation(
        chain_key: u64,
        digest: H256,
    ) -> Option<attestor_primitives::SignedAttestation<Runtime::Hash, Runtime::AccountId>> {
        pallet_attestation_poc::Pallet::<Runtime>::attestations(chain_key, digest)
    }

    /// Get the last finalized digest for a chain
    ///
    /// Returns the most recent attestation digest for the specified chain.
    /// Currently unused but kept for potential future optimizations.
    #[allow(dead_code)]
    fn last_digest(chain_key: u64) -> Option<H256> {
        pallet_attestation_poc::Pallet::<Runtime>::last_attestation_digest(chain_key)
    }

    /// Get the last checkpoint for a chain
    ///
    /// Returns the most recent checkpoint for the specified chain.
    /// Checkpoints are intermediate consensus points between full attestations.
    /// Currently unused but kept for potential future optimizations.
    #[allow(dead_code)]
    fn last_checkpoint(chain_key: u64) -> Option<attestor_primitives::AttestationCheckpoint> {
        pallet_attestation_poc::Pallet::<Runtime>::last_checkpoint(chain_key)
    }

    /// Check if a digest corresponds to a checkpoint
    ///
    /// Returns the block number if the digest matches a checkpoint,
    /// None otherwise. Used as a fallback when attestation lookup fails.
    fn get_checkpoint(chain_key: u64, digest: H256) -> Option<u64> {
        pallet_attestation_poc::Pallet::<Runtime>::checkpoints(chain_key, digest)
    }
}
