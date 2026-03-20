use attestor_primitives::block::ContinuityProof;
use fp_evm::{ExitError, ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use precompile_utils::prelude::*;
use sp_core::H256;

use crate::verify::CONTINUITY_BLOCK_HASH_COST;
use crate::BlockProverPrecompile;

/// Error type for continuity chain validation
#[derive(Debug, Clone, PartialEq)]
pub enum ContinuityVerificationError {
    /// Continuity chain doesn't have enough blocks
    InsufficientBlocks,
    /// Continuity chain does not reach query height
    ChainDoesNotReachQueryHeight,
    /// Continuity chain does not end at a valid attestation or checkpoint
    NoMatchingAttestationOrCheckpoint,
}

impl ContinuityVerificationError {
    /// Get the error message
    pub fn message(&self) -> &'static str {
        match self {
            Self::InsufficientBlocks => "Continuity chain cannot be empty",
            Self::ChainDoesNotReachQueryHeight => "Continuity chain does not reach query height",
            Self::NoMatchingAttestationOrCheckpoint => {
                "Continuity proof does not match attestation or checkpoint"
            }
        }
    }
}

fn continuity_revert(err: ContinuityVerificationError) -> PrecompileFailure {
    PrecompileFailure::Revert {
        output: crate::encode_revert_message(err.message()),
        exit_status: ExitRevert::Reverted,
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
    /// Verify the continuity chain of block attestations
    ///
    /// Validates that the continuity chain:
    /// 1. Forms an unbroken chain of blocks (digests computed sequentially from roots)
    /// 2. Covers the query height
    /// 3. Ends at a valid attestation or checkpoint (consensus point)
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting
    /// - `continuity_proof`: Continuity proof to validate (contains roots, digests computed on-chain)
    /// - `start_block_number`: Starting block number (roots[0] is at this height)
    /// - `chain_key`: Chain key identifier
    /// - `height`: Query block height (used to verify chain covers query height)
    ///
    /// # Returns
    /// - `Ok(())`: Chain is valid and ends at attestation/checkpoint
    /// - `Err(PrecompileFailure::Revert)`: Logical continuity error (encoded message)
    /// - `Err(PrecompileFailure::Error { OutOfGas })`: Gas exhaustion (from `record_cost` or storage reads)
    ///
    /// # Gas Costs
    /// - `CONTINUITY_BLOCK_HASH_COST` (48) per block in the chain (charged upfront)
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
    ) -> EvmResult<()> {
        // Require at least 1 root (empty continuity proof is invalid)
        if continuity_proof.roots.is_empty() {
            return Err(continuity_revert(
                ContinuityVerificationError::InsufficientBlocks,
            ));
        }

        // Validate the continuity chain reaches the query height (fail early before digest computation)
        let final_block_number = start_block_number + (continuity_proof.roots.len() - 1) as u64;

        if final_block_number < height {
            debug!(
                "❌ Continuity chain ends at block {final_block_number}, but query requires block {height}"
            );
            return Err(continuity_revert(
                ContinuityVerificationError::ChainDoesNotReachQueryHeight,
            ));
        }

        // Charge gas for continuity block verification upfront
        let total_continuity_gas = CONTINUITY_BLOCK_HASH_COST
            .checked_mul(continuity_proof.roots.len() as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(total_continuity_gas)?;

        // Compute final digest by hashing through all blocks
        let final_digest = continuity_proof.compute_continuity_digest(start_block_number);

        // Check if there's an attestation or checkpoint at this exact block height with matching digest
        // Gas is charged inside get_attestation() and get_checkpoint()
        let attestation_matches = Self::get_attestation(handle, chain_key, final_digest)?
            .map(|a| a.attestation.header_number == final_block_number)
            .unwrap_or(false);

        // Only check checkpoint if attestation doesn't match (saves storage read in common case)
        let checkpoint_matches = if attestation_matches {
            false // No need to check checkpoint if attestation matches
        } else {
            Self::get_checkpoint(handle, chain_key, final_block_number)?
                .map(|digest| digest == final_digest)
                .unwrap_or(false)
        };

        // Combine attestation_matches and checkpoint_matches into a single boolean
        // since they're always used together
        let they_match = attestation_matches || checkpoint_matches;

        // Special case: If the continuity chain ends at query height and query height
        // is a checkpoint/attestation, that's valid (allows queries at checkpoint/attestation heights)
        if final_block_number == height && they_match {
            debug!(
                "✅ Continuity chain ends at query height {height} which is a checkpoint/attestation"
            );
        } else if !they_match {
            // Chain must end at a checkpoint/attestation
            debug!(
                "❌ Continuity chain ends at block {final_block_number} with digest {final_digest:?}, but no matching attestation or checkpoint found at that height"
            );
            return Err(continuity_revert(
                ContinuityVerificationError::NoMatchingAttestationOrCheckpoint,
            ));
        }

        Ok(())
    }
}
