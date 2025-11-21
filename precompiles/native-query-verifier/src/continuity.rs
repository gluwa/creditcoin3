use attestor_primitives::{block::Block, query::Query};
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use precompiles_primitives::GAS_STORAGE_LOOKUP;
use sp_core::H256;

use crate::NativeQueryVerifierPrecompile;

// Gas cost constants
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
    /// Find query block index in continuity chain using optimized search
    ///
    /// Returns the index of the block with the given height, or None if not found.
    /// First tries computed index (O(1) for sequential blocks), then falls back
    /// to binary search if blocks are sorted.
    pub(crate) fn find_query_block_index(
        continuity_blocks: &[Block],
        query_height: u64,
    ) -> Option<usize> {
        // Try computed index first (O(1) for sequential blocks)
        if let (Some(first_block), Some(last_block)) =
            (continuity_blocks.first(), continuity_blocks.last())
        {
            let first_block_num = first_block.block_number;
            let last_block_num = last_block.block_number;

            if query_height >= first_block_num && query_height <= last_block_num {
                let index = (query_height - first_block_num) as usize;
                if index < continuity_blocks.len() {
                    let block = &continuity_blocks[index];
                    if block.block_number == query_height {
                        return Some(index);
                    }
                }
            }
        }

        // Fall back to binary search (O(log n)) - assumes blocks are sorted
        continuity_blocks
            .binary_search_by_key(&query_height, |b| b.block_number)
            .ok()
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
    /// - `continuity_blocks`: Chain of blocks to validate (converted from ContinuityProof internally)
    /// - `query`: Query specification (used to verify chain covers query height)
    ///
    /// # Note
    /// This function receives Vec<Block> which is converted from ContinuityProof in the caller.
    /// The ContinuityProof structure optimizes calldata by removing block_number and prev_digest
    /// from individual blocks, but internally we work with full Block structures for verification.
    ///
    /// # Returns
    /// - `Ok(true)`: Chain is valid and ends at attestation/checkpoint
    /// - `Err(ContinuityVerificationError)`: Structured error with status code and message
    ///
    /// # Gas Costs
    /// - Gas is charged upfront per block in `verify_query_impl` using `GAS_PER_CONTINUITY_BLOCK`
    /// - `GAS_STORAGE_LOOKUP` (2600) for attestation lookup
    /// - Additional `GAS_STORAGE_LOOKUP` for checkpoint lookup (only if attestation doesn't match)
    ///
    /// # Note
    /// For POC optimization compatibility, batch verification uses implicit block
    /// numbering where block_number can be computed from index. This optimization
    /// is implemented directly in the batch verification logic for efficiency.
    pub fn verify_continuity_chain(
        handle: &mut impl PrecompileHandle,
        continuity_blocks: &[Block],
        query: &Query,
    ) -> Result<bool, ContinuityVerificationError> {
        // Security: Always require at least 2 blocks (queryHeight-1 and queryHeight)
        // This is required to verify the query block's digest using the previous block's digest
        // POC pattern: continuity chain starts at queryHeight - 1
        if continuity_blocks.len() < 2 {
            return Err(ContinuityVerificationError::InsufficientBlocks);
        }

        // Multi-block chain: validate links between blocks
        // Start validation from the first block's prev_digest
        // This should link back to a known attestation or checkpoint
        let mut prev_digest = continuity_blocks[0].prev_digest;

        // Validate each block in the continuity chain
        for cb in continuity_blocks {
            let block_digest = cb.digest;
            let block_prev_digest = cb.prev_digest;

            // Verify the link continues exactly
            if prev_digest != block_prev_digest {
                debug!(
                    "❌ Continuity proof break: expected prev_digest {prev_digest:?}, got {block_prev_digest:?}"
                );
                return Err(ContinuityVerificationError::ChainLinkBroken);
            }

            // Verify the stored digest matches what would be computed using the prev_digest
            // This catches cases where prev_digest was wrong but digest wasn't recomputed
            let computed_digest = Block::hash_payload(&cb.block_number, &cb.root, &prev_digest);
            if computed_digest != block_digest {
                debug!(
                    "❌ Continuity proof digest mismatch: computed {computed_digest:?}, got {block_digest:?}"
                );
                return Err(ContinuityVerificationError::ChainLinkBroken);
            }

            // Update the last block digest to the current block's digest
            prev_digest = block_digest;
        }

        // Validate the continuity chain reaches the query height
        if let Some(head) = continuity_blocks.last() {
            if head.block_number < query.height {
                debug!(
                    "❌ Continuity chain ends at block {}, but query requires block {}",
                    head.block_number, query.height
                );
                return Err(ContinuityVerificationError::ChainDoesNotReachQueryHeight);
            }

            // The last block should be at a checkpoint or attestation height
            // and its digest should match
            let final_digest = head.digest;
            let final_block_number = head.block_number;

            // Charge for attestation storage lookup
            handle
                .record_cost(GAS_STORAGE_LOOKUP)
                .map_err(|_| ContinuityVerificationError::NoMatchingAttestationOrCheckpoint)?;

            // Check if there's an attestation at this exact block height with matching digest
            let attestation_matches = Self::get_attestation(query.chain_id, final_digest)
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

                Self::get_checkpoint(query.chain_id, final_block_number)
                    .map(|digest| digest == final_digest)
                    .unwrap_or(false)
            };

            // Special case: If the continuity chain ends at query.height and query.height
            // is a checkpoint/attestation, that's valid (allows queries at checkpoint/attestation heights)
            if final_block_number == query.height && (attestation_matches || checkpoint_matches) {
                debug!(
                    "✅ Continuity chain ends at query height {} which is a checkpoint/attestation",
                    query.height
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
    /// - `continuity_blocks`: Chain of blocks containing query block (converted from ContinuityProof internally)
    /// - `query`: Query specification
    /// - `merkle_root`: Merkle root from the merkle proof (must match query block root)
    ///
    /// # Note
    /// This function receives Vec<Block> which is converted from ContinuityProof in the caller.
    /// Block numbers are inferred from the ContinuityProof structure (blocks[0] = queryHeight-1).
    ///
    /// # Returns
    /// - `Ok(())`: Query block digest is valid
    /// - `Err(ContinuityVerificationError)`: Structured error with status code and message
    ///
    /// # Gas Costs
    /// - `GAS_KECCAK256_HASH` (48) for hash computation (Keccak-256 on 72 bytes)
    pub fn verify_query_block_digest(
        handle: &mut impl PrecompileHandle,
        continuity_blocks: &[Block],
        query: &Query,
        merkle_root: H256,
    ) -> Result<(), ContinuityVerificationError> {
        // Security: Always require at least 2 blocks (queryHeight-1 and queryHeight)
        // This is required to verify the query block's digest using the previous block's digest
        if continuity_blocks.len() < 2 {
            return Err(ContinuityVerificationError::InsufficientBlocks);
        }

        // Find the query block index using optimized search
        let query_block_idx = Self::find_query_block_index(continuity_blocks, query.height)
            .ok_or(ContinuityVerificationError::QueryBlockNotFound)?;

        let query_block = &continuity_blocks[query_block_idx];

        // Verify merkle root matches
        if query_block.root != merkle_root {
            return Err(ContinuityVerificationError::MerkleRootMismatch);
        }

        // Optimize: Use the query block index to directly access the previous block
        // Since blocks are sequential (queryHeight-1, queryHeight), prev is at index - 1
        if query_block_idx == 0 {
            return Err(ContinuityVerificationError::PreviousBlockNotFound);
        }
        let prev_block = &continuity_blocks[query_block_idx - 1];

        // Verify the previous block is actually at queryHeight - 1 (safety check)
        if prev_block.block_number != query.height.saturating_sub(1) {
            return Err(ContinuityVerificationError::PreviousBlockNotFound);
        }

        // Compute expected digest for query block using previous block's digest
        use attestor_primitives::block::Block as FragmentBlock;
        let expected_digest =
            FragmentBlock::hash_payload(&query.height, &query_block.root, &prev_block.digest);

        // Charge for hash computation (Keccak-256 on 72 bytes)
        handle.record_cost(GAS_KECCAK256_HASH).map_err(|_| {
            ContinuityVerificationError::DigestMismatch // Use a generic error if gas recording fails
        })?;

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
