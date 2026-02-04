#![cfg_attr(not(feature = "std"), no_std)]

//! Native Query Verifier Precompile
//!
//! This precompile provides native-speed verification of blockchain queries using:
//! - Merkle proof verification for transaction inclusion in blocks
//! - Continuity chain validation for block attestations
//!
//! The precompile is accessible at address 0x0FD2 (4050 in decimal)

use core::marker::PhantomData;
use fp_evm::{ExitError, ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use pallet_evm::AddressMapping;
use precompile_utils::{keccak256, prelude::*};
use sp_core::H256;

use attestor_primitives::block::ContinuityProof;
use ethabi::{encode, Token};
use sp_std::vec::Vec;

use crate::verify::GAS_STORAGE_LOOKUP;

// Use the TransactionMerkleProof from merkle
use merkle::TransactionMerkleProof;

/// Type alias for 10MB bounded vec constraint (10_485_760 bytes)
pub type ConstU10MB = sp_core::ConstU32<10_485_760>;

/// Helper function to encode revert messages in Solidity format
///
/// Encodes a string message as a Solidity `Error(string)` revert.
/// The output format is: [Error(string) selector (4 bytes)] + [ABI-encoded string]
///
/// # Arguments
/// * `message` - The error message to encode
///
/// # Returns
/// A byte vector containing the encoded revert message
pub fn encode_revert_message(message: &str) -> Vec<u8> {
    // Function selector for Error(string): keccak256("Error(string)")[0:4] = 0x08c379a0
    let mut revert_with_selector = [0x08, 0xc3, 0x79, 0xa0].to_vec();
    let encoded_revert = encode(&[Token::String(message.into())]);
    revert_with_selector.extend(encoded_revert);
    revert_with_selector
}

// Event selectors (keccak256 of event signatures)
// TransactionVerified(uint64 indexed,uint64 indexed,uint64)
// ChainKey (indexed), Height (indexed), TxIndex (data)
pub const SELECTOR_LOG_TRANSACTION_VERIFIED: [u8; 32] =
    keccak256!("TransactionVerified(uint64,uint64,uint64)");

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
pub struct BlockProverPrecompile<Runtime>(PhantomData<Runtime>);

// Batch queries constraint
type MaxBatchSize = sp_core::ConstU32<10>;

#[precompile_utils::precompile]
impl<Runtime> BlockProverPrecompile<Runtime>
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
    /// - `encoded_transaction`: The raw transaction data to verify
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
        "verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))"
    )]
    #[precompile::view]
    fn verify(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        height: u64,
        encoded_transaction: BoundedBytes<ConstU10MB>,
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_impl(
            handle,
            chain_key,
            height,
            encoded_transaction,
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
    /// - `encoded_transaction`: The raw transaction data to verify
    /// - `merkle_proof`: Merkle proof for transaction inclusion in the block
    /// - `continuity_proof`: Optimized continuity proof (blocks[0] is at queryHeight-1)
    ///
    /// # Returns
    /// `true` on successful verification
    ///
    /// # Events
    /// Emits `TransactionVerified(uint64,uint64,uint64)` with chain_key, height, and transactionIndex
    ///
    /// # Reverts
    /// - If continuity chain is empty or invalid
    /// - If merkle proof verification fails
    /// - If merkle root doesn't match continuity block
    /// - If query block not found in continuity chain
    /// - If continuity chain doesn't end at a valid attestation/checkpoint
    #[precompile::public(
        "verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))"
    )]
    fn verify_and_emit(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        height: u64,
        encoded_transaction: BoundedBytes<ConstU10MB>,
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_impl(
            handle,
            chain_key,
            height,
            encoded_transaction,
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
    /// * `encoded_transactions` - Transaction data for each query (must match heights length)
    /// * `merkle_proofs` - Merkle proofs for each query (must match heights length)
    /// * `shared_continuity_proof` - Shared continuity proof covering all query heights
    ///
    /// # Returns
    /// `bool` indicating whether the batch verification was successful
    ///
    /// # Reverts
    /// - If input arrays have mismatched lengths
    /// - If shared continuity chain is invalid
    #[precompile::public(
        "verify(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))"
    )]
    #[precompile::view]
    fn verify_batch(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        heights: BoundedVec<u64, MaxBatchSize>,
        encoded_transactions: BoundedVec<BoundedBytes<ConstU10MB>, MaxBatchSize>,
        merkle_proofs: BoundedVec<TransactionMerkleProof, MaxBatchSize>,
        shared_continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_batch_impl(
            handle,
            chain_key,
            heights.into(),
            encoded_transactions.into(),
            merkle_proofs.into(),
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
    /// * `encoded_transactions` - Transaction data for each query (must match heights length)
    /// * `merkle_proofs` - Merkle proofs for each query (must match queries length)
    /// * `shared_continuity_proof` - Shared continuity proof covering all query heights
    ///
    /// # Returns
    /// `bool` indicating whether the batch verification was successful
    ///
    /// # Events
    /// Emits `TransactionVerified(uint64,uint64,uint8)` for each successfully verified transaction,
    /// with chain_key, height, and transactionIndex (transaction index calculated from Merkle proof siblings)
    ///
    /// # Reverts
    /// - If input arrays have mismatched lengths
    /// - If shared continuity chain is invalid
    #[precompile::public(
        "verifyAndEmit(uint64,uint64[],bytes[],(bytes32,(bytes32,bool)[])[],(bytes32,bytes32[]))"
    )]
    fn verify_batch_and_emit(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        heights: BoundedVec<u64, MaxBatchSize>,
        encoded_transactions: BoundedVec<BoundedBytes<ConstU10MB>, MaxBatchSize>,
        merkle_proofs: BoundedVec<TransactionMerkleProof, MaxBatchSize>,
        shared_continuity_proof: ContinuityProof,
    ) -> EvmResult<bool> {
        Self::verify_batch_impl(
            handle,
            chain_key,
            heights.into(),
            encoded_transactions.into(),
            merkle_proofs.into(),
            shared_continuity_proof,
            true, // emit events (state-changing function)
        )
    }

    /// Calculate transaction index from Merkle proof siblings (view function)
    ///
    /// Reconstructs the transaction index by working from leaf to root.
    /// The `is_left` flags in siblings indicate the path taken through the tree.
    /// Siblings are stored from leaf level to root level.
    /// - If sibling is left (`is_left = true`), current node was right, so bit = 1
    /// - If sibling is right (`is_left = false`), current node was left, so bit = 0
    ///
    /// # Parameters
    /// - `merkle_proof`: Merkle proof containing root and siblings with position information
    ///
    /// # Returns
    /// The transaction index (leaf position in the Merkle tree) as uint64
    ///
    /// # Example
    /// For a Merkle proof with siblings indicating path [right, left], the transaction index
    /// would be calculated as: bit 0 = 1 (right), bit 1 = 0 (left) = index 1
    #[precompile::public("calculateTxIndex((bytes32,(bytes32,bool)[]))")]
    #[precompile::view]
    fn calculate_tx_index(
        handle: &mut impl PrecompileHandle,
        merkle_proof: TransactionMerkleProof,
    ) -> EvmResult<u64> {
        // Charge gas: base cost (10) + per iteration cost (18) * merkle path length
        let merkle_path_len = merkle_proof.siblings.len();
        let iteration_gas = crate::verify::CALCULATE_TX_INDEX_ITERATION_COST
            .checked_mul(merkle_path_len as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        let total_gas = crate::verify::CALCULATE_TX_INDEX_BASE_COST
            .checked_add(iteration_gas)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(total_gas)?;

        Self::calculate_tx_index_impl(&merkle_proof)
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
    /// Charges gas for the storage lookup.
    ///
    /// Returns `Result<Option<T>, ContinuityVerificationError>` to properly propagate
    /// gas recording errors instead of swallowing them.
    fn get_attestation(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        digest: H256,
    ) -> EvmResult<Option<attestor_primitives::SignedAttestation<Runtime::Hash, Runtime::AccountId>>>
    {
        // Charge for attestation storage lookup
        handle.record_cost(GAS_STORAGE_LOOKUP)?;
        Ok(pallet_attestation_poc::Pallet::<Runtime>::attestations(
            chain_key, digest,
        ))
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
    /// Charges gas for the storage lookup.
    ///
    /// Returns `Result<Option<T>, ContinuityVerificationError>` to properly propagate
    /// gas recording errors instead of swallowing them.
    fn get_checkpoint(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        block_number: u64,
    ) -> EvmResult<Option<H256>> {
        // Charge for checkpoint storage lookup only if we need to check it
        handle.record_cost(GAS_STORAGE_LOOKUP)?;
        Ok(pallet_attestation_poc::Pallet::<Runtime>::checkpoints(
            chain_key,
            block_number,
        ))
    }
}
