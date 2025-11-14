use attestor_primitives::{block::Block, query::Query};
use fp_evm::{ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use precompile_utils::{prelude::*, solidity::Codec};
use sp_core::H256;

use crate::{encode_revert_message, NativeQueryVerifierPrecompile};

// Gas cost constants
pub const GAS_STORAGE_LOOKUP: u64 = 2_600; // Each storage read (matches cold SLOAD)

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
    /// Verify the continuity chain of block attestations
    ///
    /// Validates that the continuity chain:
    /// 1. Forms an unbroken chain of blocks (each block's prev_digest matches previous block's digest)
    /// 2. Covers the query height
    /// 3. Ends at a valid attestation or checkpoint (consensus point)
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting
    /// - `continuity_blocks`: Chain of blocks to validate
    /// - `query`: Query specification (used to verify chain covers query height)
    ///
    /// # Returns
    /// - `Ok(true)`: Chain is valid and ends at attestation/checkpoint
    /// - `Ok(false)`: Chain has broken links
    /// - `Err`: Chain doesn't reach query height or doesn't end at consensus point
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
    ) -> Result<bool, PrecompileFailure> {
        // Should not be called with empty continuity_blocks, but check anyway
        if continuity_blocks.is_empty() {
            return Ok(false);
        }

        // Start validation from the first block's prev_digest
        // This should link back to a known attestation or checkpoint
        let mut prev_digest = if let Some(tail) = continuity_blocks.first() {
            tail.prev_digest
        } else {
            // Empty continuity chain - shouldn't happen but return false
            return Ok(false);
        };

        // Validate each block in the continuity chain
        for cb in continuity_blocks {
            let block_digest = cb.digest;
            let block_prev_digest = cb.prev_digest;

            // Verify the link continues exactly
            if prev_digest != block_prev_digest {
                debug!(
                    "❌ Continuity proof break: expected prev_digest {prev_digest:?}, got {block_prev_digest:?}"
                );
                return Ok(false);
            }
            // Charge for the block verification
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

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
                return Err(PrecompileFailure::Revert {
                    output: encode_revert_message("Continuity chain does not reach query height"),
                    exit_status: ExitRevert::Reverted,
                });
            }

            // The last block should be at a checkpoint or attestation height
            // and its digest should match
            let final_digest = head.digest;
            let final_block_number = head.block_number;

            // Charge for storage lookup
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

            // Check if there's an attestation at this exact block height with matching digest
            let attestation_matches = Self::get_attestation(query.chain_id, final_digest)
                .map(|a| a.attestation.header_number == final_block_number)
                .unwrap_or(false);

            // Check if there's a checkpoint at this exact block height with matching digest
            let checkpoint_matches = Self::get_checkpoint(query.chain_id, final_digest)
                .map(|block_num| block_num == final_block_number)
                .unwrap_or(false);

            if !attestation_matches && !checkpoint_matches {
                debug!(
                "❌ Continuity chain ends at block {final_block_number} with digest {final_digest:?}, but no matching attestation or checkpoint found at that height"
            );
                return Err(PrecompileFailure::Revert {
                    output: encode_revert_message("Continuity proof does not match checkpoint"),
                    exit_status: ExitRevert::Reverted,
                });
            }
        }

        Ok(true)
    }
}
