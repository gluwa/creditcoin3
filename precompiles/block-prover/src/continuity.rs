use attestor_primitives::block::{Block, ContinuityProof};
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use sp_core::H256;

use crate::BlockProverPrecompile;

// Gas cost constants
/// Cost of each storage read (matches cold SLOAD) in gas.
pub const GAS_STORAGE_LOOKUP: u64 = 2_600;
pub const GAS_KECCAK256_HASH: u64 = 48; // Keccak-256 hash cost: 30 base + 6 per word (72 bytes = 3 words)

/// Error type for continuity verification (both query block digest and chain validation)
#[derive(Debug, Clone, PartialEq)]
pub enum ContinuityVerificationError {
    /// Continuity chain doesn't have enough blocks
    InsufficientBlocks,
    /// Query block not found in continuity chain
    QueryBlockNotFound,
    /// Merkle root doesn't match query block root
    MerkleRootMismatch,
    /// Previous block (queryHeight-1) not found
    PreviousBlockNotFound,
    /// Query block digest verification failed
    DigestMismatch,
    /// Continuity chain does not reach query height
    ChainDoesNotReachQueryHeight,
    /// Continuity chain does not end at a valid attestation or checkpoint
    NoMatchingAttestationOrCheckpoint,
    /// Continuity chain has broken links between blocks
    ChainLinkBroken,
}

impl ContinuityVerificationError {
    /// Get the status code for this error
    pub fn status(&self) -> u8 {
        match self {
            Self::MerkleRootMismatch => 4, // MerkleRootMismatch
            _ => 2,                        // ContinuityChainInvalid
        }
    }

    /// Get the error message
    pub fn message(&self) -> &'static str {
        match self {
            Self::InsufficientBlocks => {
                "Continuity chain must contain at least 2 blocks (queryHeight-1 and queryHeight)"
            }
            Self::QueryBlockNotFound => "Query block not found in continuity chain",
            Self::MerkleRootMismatch => "Merkle root mismatch",
            Self::PreviousBlockNotFound => {
                "Previous block (queryHeight-1) not found in continuity chain"
            }
            Self::DigestMismatch => "Query block digest verification failed",
            Self::ChainDoesNotReachQueryHeight => "Continuity chain does not reach query height",
            Self::NoMatchingAttestationOrCheckpoint => {
                "Continuity proof does not match attestation or checkpoint"
            }
            Self::ChainLinkBroken => "Continuity chain has broken links",
        }
    }
}

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
    /// Find query block index in continuity proof using optimized search
    ///
    /// Returns the index of the block with the given height, or None if not found.
    /// Uses computed index (O(1)) since blocks are sequential starting from start_block_number.
    pub(crate) fn find_query_block_index(
        continuity_proof: &ContinuityProof,
        start_block_number: u64,
        query_height: u64,
    ) -> Option<usize> {
        if continuity_proof.blocks.is_empty() {
            return None;
        }

        let first_block_num = start_block_number;
        let last_block_num = start_block_number + (continuity_proof.blocks.len() - 1) as u64;

        if query_height >= first_block_num && query_height <= last_block_num {
            let index = (query_height - first_block_num) as usize;
            if index < continuity_proof.blocks.len() {
                return Some(index);
            }
        }

        None
    }
    /// Verify the continuity chain of block attestations
    ///
    /// Validates that the continuity chain:
    /// 1. Forms an unbroken chain of blocks (each block's prev_digest matches previous block's digest)
    /// 2. Covers the query height
    /// 3. Ends at a valid attestation or checkpoint (consensus point)
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting
    /// - `continuity_proof`: Continuity proof to validate (optimized structure)
    /// - `start_block_number`: Starting block number (blocks[0] is at this height)
    /// - `chain_key`: Chain key identifier
    /// - `height`: Query block height (used to verify chain covers query height)
    ///
    /// # Returns
    /// - `Ok(true)`: Chain is valid and ends at attestation/checkpoint
    /// - `Err(ContinuityVerificationError)`: Structured error with status code and message
    ///
    /// # Gas Costs
    /// - Gas is charged upfront per block in `verify_impl` using `GAS_PER_CONTINUITY_BLOCK`
    /// - `GAS_STORAGE_LOOKUP` (2600) for attestation lookup
    /// - Additional `GAS_STORAGE_LOOKUP` for checkpoint lookup (only if attestation doesn't match)
    ///
    /// # Optimization
    /// Instead of comparing each intermediate digest, we hash through the entire chain
    /// and only compare the final hash. If the final hash matches the attestation/checkpoint,
    /// all intermediate hashes must be correct (hashing is deterministic).
    pub fn verify_continuity_chain(
        handle: &mut impl PrecompileHandle,
        continuity_proof: &ContinuityProof,
        start_block_number: u64,
        chain_key: u64,
        height: u64,
    ) -> Result<bool, ContinuityVerificationError> {
        // Security: Always require at least 2 blocks (queryHeight-1 and queryHeight)
        // This is required to verify the query block's digest using the previous block's digest
        // POC pattern: continuity chain starts at queryHeight - 1
        if continuity_proof.blocks.len() < 2 {
            return Err(ContinuityVerificationError::InsufficientBlocks);
        }

        // Multi-block chain: hash through all blocks sequentially
        // Start validation from the lower_endpoint_digest (prev_digest of first block)
        let mut prev_digest = continuity_proof.lower_endpoint_digest;

        // Hash through all blocks without intermediate comparisons
        // If any intermediate value is wrong, the final hash will be wrong
        for (idx, cb) in continuity_proof.blocks.iter().enumerate() {
            let block_number = start_block_number + idx as u64;

            // Compute digest for this block using previous block's digest
            // No comparison here - we'll only compare the final hash
            prev_digest = Block::hash_payload(&block_number, &cb.merkle_root, &prev_digest);
        }

        // Validate the continuity chain reaches the query height
        if let Some(head) = continuity_proof.blocks.last() {
            let final_block_number =
                start_block_number + (continuity_proof.blocks.len() - 1) as u64;

            if final_block_number < height {
                debug!(
                    "❌ Continuity chain ends at block {final_block_number}, but query requires block {height}"
                );
                return Err(ContinuityVerificationError::ChainDoesNotReachQueryHeight);
            }

            // Now verify the final computed digest matches the stored digest
            // This is the only comparison we need - if this matches, all intermediate
            // hashes must have been correct (due to deterministic hashing)
            if prev_digest != head.digest {
                debug!(
                    "❌ Continuity proof digest mismatch at final block {final_block_number}: computed {prev_digest:?}, got {:?}",
                    head.digest
                );
                return Err(ContinuityVerificationError::ChainLinkBroken);
            }

            // The last block should be at a checkpoint or attestation height
            // and its digest should match
            let final_digest = head.digest;

            // Charge for attestation storage lookup
            handle
                .record_cost(GAS_STORAGE_LOOKUP)
                .map_err(|_| ContinuityVerificationError::NoMatchingAttestationOrCheckpoint)?;

            // Check if there's an attestation at this exact block height with matching digest
            let attestation_matches = Self::get_attestation(chain_key, final_digest)
                .map(|a| a.attestation.header_number == final_block_number)
                .unwrap_or(false);

            // Only check checkpoint if attestation doesn't match (saves storage read in common case)
            let checkpoint_matches = if attestation_matches {
                false // No need to check checkpoint if attestation matches
            } else {
                // Charge for checkpoint storage lookup only if we need to check it
                handle
                    .record_cost(GAS_STORAGE_LOOKUP)
                    .map_err(|_| ContinuityVerificationError::NoMatchingAttestationOrCheckpoint)?;

                Self::get_checkpoint(chain_key, final_block_number)
                    .map(|digest| digest == final_digest)
                    .unwrap_or(false)
            };

            // Special case: If the continuity chain ends at query height and query height
            // is a checkpoint/attestation, that's valid (allows queries at checkpoint/attestation heights)
            if final_block_number == height && (attestation_matches || checkpoint_matches) {
                debug!(
                    "✅ Continuity chain ends at query height {height} which is a checkpoint/attestation"
                );
            } else if !attestation_matches && !checkpoint_matches {
                // Chain must end at a checkpoint/attestation
                debug!(
                    "❌ Continuity chain ends at block {final_block_number} with digest {final_digest:?}, but no matching attestation or checkpoint found at that height"
                );
                return Err(ContinuityVerificationError::NoMatchingAttestationOrCheckpoint);
            }
        }

        Ok(true)
    }

    /// Verify the query block's digest is computed correctly
    ///
    /// Security: This prevents sending fake roots by requiring the query block's digest
    /// to be computed from the previous block's digest. Following POC pattern where
    /// continuity chain starts at queryHeight - 1.
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting
    /// - `continuity_proof`: Continuity proof containing query block (optimized structure)
    /// - `start_block_number`: Starting block number (blocks[0] is at this height)
    /// - `height`: Query block height
    /// - `merkle_root`: Merkle root from the merkle proof (must match query block root)
    ///
    /// # Returns
    /// - `Ok(())`: Query block digest is valid
    /// - `Err(ContinuityVerificationError)`: Structured error with status code and message
    ///
    /// # Gas Costs
    /// - `GAS_KECCAK256_HASH` (48) for hash computation (Keccak-256 on 72 bytes)
    pub fn verify_query_block_digest(
        handle: &mut impl PrecompileHandle,
        continuity_proof: &ContinuityProof,
        start_block_number: u64,
        height: u64,
        merkle_root: H256,
    ) -> Result<(), ContinuityVerificationError> {
        // Security: Always require at least 2 blocks (queryHeight-1 and queryHeight)
        // This is required to verify the query block's digest using the previous block's digest
        if continuity_proof.blocks.len() < 2 {
            return Err(ContinuityVerificationError::InsufficientBlocks);
        }

        // Find the query block index using optimized search
        let query_block_idx =
            Self::find_query_block_index(continuity_proof, start_block_number, height)
                .ok_or(ContinuityVerificationError::QueryBlockNotFound)?;

        let query_block = &continuity_proof.blocks[query_block_idx];

        // Verify merkle root matches
        if query_block.merkle_root != merkle_root {
            return Err(ContinuityVerificationError::MerkleRootMismatch);
        }

        // Optimize: Use the query block index to directly access the previous block
        // Since blocks are sequential (queryHeight-1, queryHeight), prev is at index - 1
        if query_block_idx == 0 {
            return Err(ContinuityVerificationError::PreviousBlockNotFound);
        }

        // Verify the previous block is actually at queryHeight - 1 (safety check)
        let prev_block_number = start_block_number + (query_block_idx - 1) as u64;
        if prev_block_number != height.saturating_sub(1) {
            return Err(ContinuityVerificationError::PreviousBlockNotFound);
        }

        // Reconstruct prev_digest for the query block
        // The prev_digest of the query block is the digest of the previous block
        let prev_digest = if query_block_idx == 0 {
            // Query block is the first block, so prev_digest is lower_endpoint_digest
            continuity_proof.lower_endpoint_digest
        } else {
            // Query block is not the first, so prev_digest is the digest of the previous block
            continuity_proof.blocks[query_block_idx - 1].digest
        };

        // Charge for hash computation BEFORE computing (security: prevent out-of-gas attacks)
        handle.record_cost(GAS_KECCAK256_HASH).map_err(|_| {
            ContinuityVerificationError::DigestMismatch // Use a generic error if gas recording fails
        })?;

        // Compute expected digest for query block using previous block's digest
        let expected_digest = Block::hash_payload(&height, &query_block.merkle_root, &prev_digest);

        // Verify computed digest matches the query block's digest
        if expected_digest != query_block.digest {
            debug!(
                "Query block digest verification failed: expected {:?}, got {:?}",
                expected_digest, query_block.digest
            );
            return Err(ContinuityVerificationError::DigestMismatch);
        }

        Ok(())
    }
}
