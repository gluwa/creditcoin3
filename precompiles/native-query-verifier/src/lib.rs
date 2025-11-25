#![cfg_attr(not(feature = "std"), no_std)]

//! Native Query Verifier Precompile
//!
//! This precompile provides native-speed verification of blockchain queries using:
//! - Merkle proof verification for transaction inclusion in blocks
//! - Continuity chain validation for block attestations
//!
//! The precompile is accessible at address 0x0FD2 (4050 in decimal)

use attestor_primitives::block::ContinuityProof;
use core::marker::PhantomData;
use ethabi::{encode, Token};
use fp_evm::{ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use pallet_evm::AddressMapping;
use precompile_utils::{keccak256, prelude::*};
use sp_core::H256;
use sp_std::vec::Vec;

// Use the QueryMerkleProof from the mmr crate
use mmr::query_proof::QueryMerkleProof;

// Type alias for compatibility
type MerkleProof = QueryMerkleProof;

// Event selectors (keccak256 of event signatures)
// TransactionVerified(uint64 indexed,uint64 indexed,uint8)
// ChainKey (indexed), Height (indexed), TxIndex (data)
pub const SELECTOR_LOG_TRANSACTION_VERIFIED: [u8; 32] =
    keccak256!("TransactionVerified(uint64,uint64,uint8)");

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

mod continuity;
mod verify;

/// Native Query Verifier Precompile
///
/// Provides efficient, native-speed verification of blockchain queries by validating:
/// 1. Merkle proofs for transaction inclusion in blocks
/// 2. Continuity chains linking blocks to attested checkpoints
///
/// This precompile enables trustless cross-chain data verification without requiring
/// the full blockchain state, making it ideal for bridges and oracles.
pub struct NativeQueryVerifierPrecompile<Runtime>(PhantomData<Runtime>);

// Size constraints
type ConstU10MB = sp_core::ConstU32<10_485_760>; // Type alias for bounded vec

// Batch queries constraint
type MaxBatchSize = sp_core::ConstU32<10>;

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
    /// This is a read-only view function that doesn't emit events. It charges the same gas
    /// as the non-view function but doesn't modify state or emit logs.
    /// Useful for off-chain verification or when events are not needed.
    ///
    /// # Parameters
    /// - `chain_key`: The chain key
    /// - `height`: The block height
    /// - `tx_data`: The raw transaction data to verify
    /// - `merkle_proof`: Merkle proof for transaction inclusion in the block
    /// - `continuity_proof`: Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// `true` on successful verification
    ///
    /// # Reverts
    /// - If continuity chain is empty or invalid
    /// - If merkle proof verification fails
    /// - If merkle root doesn't match continuity block
    /// - If query block not found in continuity chain
    /// - If continuity chain doesn't end at a valid attestation/checkpoint
    ///
    /// Note: This function does not emit events. For event emissions, use verifyAndEmit() instead.
    #[precompile::public(
        "verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,(bytes32,bytes32)[]))"
    )]
    #[precompile::view]
    fn verify(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        height: u64,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_impl(
            handle,
            chain_key,
            height,
            tx_data,
            merkle_proof,
            continuity_proof,
            false, // don't emit events (view function)
        )
    }

    /// Verify a blockchain query with Merkle proof and continuity chain
    ///
    /// This is the state-changing version that emits a `TransactionVerified` event on success.
    /// Reverts on failure, returns true on success.
    ///
    /// # Parameters
    /// - `chain_key`: The chain key
    /// - `height`: The block height
    /// - `tx_data`: The raw transaction data to verify
    /// - `merkle_proof`: Merkle proof for transaction inclusion in the block
    /// - `continuity_proof`: Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// `true` on successful verification
    ///
    /// # Events
    /// Emits `TransactionVerified(uint64,uint64,uint8)` with chain_key, height, and txIndex
    ///
    /// # Reverts
    /// - If continuity chain is empty or invalid
    /// - If merkle proof verification fails
    /// - If merkle root doesn't match continuity block
    /// - If query block not found in continuity chain
    /// - If continuity chain doesn't end at a valid attestation/checkpoint
    #[precompile::public("verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,(bytes32,bytes32)[]))")]
    fn verify_and_emit(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        height: u64,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_impl(
            handle,
            chain_key,
            height,
            tx_data,
            merkle_proof,
            continuity_proof,
            true, // emit events (state-changing function)
        )
    }

    /// Verify a batch of queries with shared continuity proof (view function)
    ///
    /// This is a read-only version that doesn't emit events. Optimized for batch
    /// verification by validating the continuity chain once and reusing it for all queries.
    /// This can save ~40% gas compared to individual verifications.
    ///
    /// # Arguments
    /// * `chain_key` - Chain key
    /// * `heights` - Vector of block heights to verify
    /// * `tx_data_array` - Transaction data for each query (must match heights length)
    /// * `merkle_proofs` - Merkle proofs for each query (must match heights length)
    /// * `shared_continuity_proof` - Shared continuity proof covering all query heights
    ///
    /// # Returns
    /// `bool` indicating whether the batch verification was successful
    ///
    /// # Reverts
    /// - If input arrays have mismatched lengths
    /// - If shared continuity chain is invalid
    #[precompile::public("verify(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,(bytes32,bytes32)[]))")]
    #[precompile::view]
    fn verify_batch(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        heights: Vec<u64>,
        tx_data_array: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<MerkleProof>,
        shared_continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_batch_impl(
            handle,
            chain_key,
            heights,
            tx_data_array,
            merkle_proofs,
            shared_continuity_proof,
            false, // don't emit events (view function)
        )
    }

    /// Verify a batch of queries with shared continuity proof
    ///
    /// This is the state-changing version that emits a `TransactionVerified` event for each successful transaction.
    /// Optimized for batch verification:
    /// 1. Validates the continuity chain once (shared across all queries)
    /// 2. For each query, verifies:
    ///    - Merkle proof for transaction inclusion
    ///    - Query block exists in continuity chain
    ///    - Merkle root matches the continuity block
    ///
    /// # Arguments
    /// * `chain_key` - Chain key
    /// * `heights` - Vector of block heights to verify
    /// * `tx_data_array` - Transaction data for each query (must match heights length)
    /// * `merkle_proofs` - Merkle proofs for each query (must match queries length)
    /// * `shared_continuity_proof` - Shared continuity proof covering all query heights
    ///
    /// # Returns
    /// `bool` indicating whether the batch verification was successful
    ///
    /// # Events
    /// Emits `TransactionVerified(uint64,uint64,uint8)` for each successfully verified transaction,
    /// with chain_key, height, and txIndex (transaction index calculated from Merkle proof siblings)
    ///
    /// # Reverts
    /// - If input arrays have mismatched lengths
    /// - If shared continuity chain is invalid
    #[precompile::public("verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,(bytes32,bytes32)[]))")]
    fn verify_batch_and_emit(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        heights: BoundedVec<u64, MaxBatchSize>,
        tx_data_array: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<MerkleProof>,
        shared_continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_batch_impl(
            handle,
            chain_key,
            heights.into(),
            tx_data_array,
            merkle_proofs,
            shared_continuity_proof,
            true, // emit events (state-changing function)
        )
    }

    /// Generic handler for verification failures
    ///
    /// Returns a revert with a descriptive error message.
    /// No events are emitted on failure (as per design requirement).
    pub fn revert_with_message(message: &str) -> EvmResult<bool> {
        // Revert with the error message
        Err(PrecompileFailure::Revert {
            output: encode_revert_message(message),
            exit_status: ExitRevert::Reverted,
        })
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
    /// Returns the digest if the block number matches a checkpoint,
    /// None otherwise. Used as a fallback when attestation lookup fails.
    fn get_checkpoint(chain_key: u64, block_number: u64) -> Option<H256> {
        pallet_attestation_poc::Pallet::<Runtime>::checkpoints(chain_key, block_number)
    }
}
