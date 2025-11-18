use attestor_primitives::{block::Block, query::Query};
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use precompile_utils::{prelude::*, solidity::Codec};
use sp_core::H256;

use crate::NativeQueryVerifierPrecompile;

// Gas cost constants
pub const GAS_STORAGE_LOOKUP: u64 = 2_600; // Each storage read (matches cold SLOAD)

/// Error type for continuity verification (both query block digest and chain validation)
#[derive(Debug, Clone)]
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

/// A segment of extracted data from the verified transaction
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct ResultSegment {
    /// Offset in the transaction data
    pub offset: u64,
    /// Extracted bytes at this offset
    pub bytes: H256,
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
    /// Find query block in continuity chain using optimized search
    ///
    /// First tries computed index (O(1) for sequential blocks), then falls back
    /// to binary search if blocks are sorted. Assumes blocks are sorted if
    /// computed index fails.
    pub(crate) fn find_query_block(
        continuity_blocks: &[Block],
        query_height: u64,
    ) -> Option<&Block> {
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
                        return Some(block);
                    }
                }
            }
        }

        // Fall back to binary search (O(log n)) - assumes blocks are sorted
        continuity_blocks
            .binary_search_by_key(&query_height, |b| b.block_number)
            .ok()
            .map(|idx| &continuity_blocks[idx])
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
    /// - `GAS_STORAGE_LOOKUP` (2600) per block validation
    /// - Additional `GAS_STORAGE_LOOKUP` for attestation/checkpoint lookups
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

            // Charge for the block verification
            handle.record_cost(GAS_STORAGE_LOOKUP).map_err(|_| {
                ContinuityVerificationError::ChainLinkBroken // Use a generic error if gas recording fails
            })?;

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

            // Charge for storage lookup
            handle
                .record_cost(GAS_STORAGE_LOOKUP)
                .map_err(|_| ContinuityVerificationError::NoMatchingAttestationOrCheckpoint)?;

            // Check if there's an attestation at this exact block height with matching digest
            let attestation_matches = Self::get_attestation(query.chain_id, final_digest)
                .map(|a| a.attestation.header_number == final_block_number)
                .unwrap_or(false);

            // Check if there's a checkpoint at this exact block height with matching digest
            let checkpoint_matches = Self::get_checkpoint(query.chain_id, final_digest)
                .map(|block_num| block_num == final_block_number)
                .unwrap_or(false);

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

        // Find the query block using optimized search
        let query_block = Self::find_query_block(continuity_blocks, query.height)
            .ok_or(ContinuityVerificationError::QueryBlockNotFound)?;

        // Verify merkle root matches
        if query_block.root != merkle_root {
            return Err(ContinuityVerificationError::MerkleRootMismatch);
        }

        // Find the previous block (queryHeight - 1) using optimized search
        let prev_block = Self::find_query_block(continuity_blocks, query.height.saturating_sub(1))
            .ok_or(ContinuityVerificationError::PreviousBlockNotFound)?;

        // Compute expected digest for query block using previous block's digest
        use attestor_primitives::block::Block as FragmentBlock;
        let expected_digest =
            FragmentBlock::hash_payload(&query.height, &query_block.root, &prev_block.digest);

        // Verify computed digest matches the query block's digest
        if expected_digest != query_block.digest {
            debug!(
                "Query block digest verification failed: expected {:?}, got {:?}",
                expected_digest, query_block.digest
            );
            return Err(ContinuityVerificationError::DigestMismatch);
        }

        // Charge for the verification
        handle.record_cost(GAS_STORAGE_LOOKUP).map_err(|_| {
            ContinuityVerificationError::DigestMismatch // Use a generic error if gas recording fails
        })?;

        Ok(())
    }
}
