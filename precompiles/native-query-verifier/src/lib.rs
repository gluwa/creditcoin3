#![cfg_attr(not(feature = "std"), no_std)]

//! Native Query Verifier Precompile
//!
//! This precompile provides native-speed verification of blockchain queries using:
//! - Merkle proof verification for transaction inclusion
//! - Continuity chain validation for block attestations
//! - Data extraction from verified transactions
//!
//! The precompile is accessible at address 0x0BEA (3050 in decimal)

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
use precompile_utils::{prelude::*, solidity::Codec};
use sp_core::H256;
use sp_std::vec::Vec;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_full_coverage;
#[cfg(test)]
mod tests_gas_security;

/// Native Query Verifier Precompile
/// Provides efficient, native-speed verification of blockchain queries
pub struct NativeQueryVerifierPrecompile<Runtime>(PhantomData<Runtime>);

// Gas cost constants
// Based on realistic Solidity implementation costs with precompile efficiency gains:
// - Base transaction: 21,000 gas
// - Keccak256 in Solidity: ~30 + 6/word, in precompile: ~10x faster
// - SLOAD: 2,100 (warm) / 2,600 (cold)
const GAS_BASE_VERIFY: u64 = 21_000; // Base transaction cost (matches Ethereum standard)
const GAS_PER_TX_BYTE: u64 = 16; // Per byte cost for transaction data (matches calldata cost)
const GAS_PER_SIBLING: u64 = 200; // Per Merkle sibling verification (native efficiency vs ~166 in Solidity)
const GAS_PER_CONTINUITY_BLOCK: u64 = 3_000; // Per block verification (storage + hash check)
const GAS_STORAGE_LOOKUP: u64 = 2_600; // Each storage read (matches cold SLOAD)
const WEIGHT_MERKLE_VERIFY: u64 = 100_000; // Merkle verification work
const WEIGHT_CONTINUITY_VERIFY: u64 = 50_000; // Continuity verification work

// Size constraints
type ConstU10MB = sp_core::ConstU32<10485760>; // 10 MB max for tx data

/// Merkle proof structure containing root and sibling hashes
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct MerkleProof {
    /// The Merkle root hash
    pub root: H256,
    /// Sibling hashes for the Merkle path
    pub siblings: Vec<H256>,
}

/// ContinuityBlock for Solidity ABI compatibility
/// Since Block already has the right structure, we just need a wrapper for Codec implementation
#[derive(Debug, Clone, Codec)]
pub struct ContinuityBlock {
    pub block_number: u64,
    pub root: H256,
    pub prev_digest: H256,
    pub digest: H256,
}

impl From<Block> for ContinuityBlock {
    fn from(block: Block) -> Self {
        Self {
            block_number: block.block_number,
            root: block.root,
            prev_digest: block.prev_digest,
            digest: block.digest,
        }
    }
}

impl From<ContinuityBlock> for Block {
    fn from(cb: ContinuityBlock) -> Self {
        Block {
            block_number: cb.block_number,
            root: cb.root,
            prev_digest: cb.prev_digest,
            digest: cb.digest,
        }
    }
}

/// Result of query verification
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct QueryVerificationResult {
    /// Verification status: 0 = Success, 1 = MerkleProofInvalid, 2 = ContinuityChainInvalid, 3 = DataExtractionError
    pub status: u8,
    /// Extracted data segments from the transaction
    pub result_segments: Vec<ResultSegment>,
}

/// A segment of extracted data from the verified transaction
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct ResultSegment {
    /// Offset in the transaction data
    pub offset: u64,
    /// Extracted bytes at this offset
    pub bytes: H256,
}

/// Helper function to encode revert messages in Solidity format
fn encode_revert_message(message: &str) -> Vec<u8> {
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
    /// Get an attestation from storage by chain key and digest
    fn get_attestation(
        chain_key: u64,
        digest: H256,
    ) -> Option<attestor_primitives::SignedAttestation<Runtime::Hash, Runtime::AccountId>> {
        pallet_attestation_poc::Pallet::<Runtime>::attestations(chain_key, digest)
    }

    /// Get the last finalized digest for a chain
    fn last_digest(chain_key: u64) -> Option<H256> {
        pallet_attestation_poc::Pallet::<Runtime>::last_attestation_digest(chain_key)
    }

    /// Get the last checkpoint for a chain
    fn last_checkpoint(chain_key: u64) -> Option<attestor_primitives::AttestationCheckpoint> {
        pallet_attestation_poc::Pallet::<Runtime>::last_checkpoint(chain_key)
    }

    /// Check if a digest corresponds to a checkpoint
    fn get_checkpoint(chain_key: u64, digest: H256) -> Option<u64> {
        pallet_attestation_poc::Pallet::<Runtime>::checkpoints(chain_key, digest)
    }
    /// Verify a blockchain query with Merkle proof and continuity chain
    ///
    /// # Parameters
    /// - `query`: The query specification (chain, block height, tx index, layout)
    /// - `tx_data`: The raw transaction data to verify
    /// - `merkle_proof`: Merkle proof for transaction inclusion
    /// - `continuity_chain`: Chain of block attestations for continuity
    ///
    /// # Returns
    /// `QueryVerificationResult` with status and extracted data segments
    #[precompile::public("verifyQuery((uint64,uint64,uint64,(uint64,uint64)[]),bytes,(bytes32,bytes32[]),(uint64,bytes32,bytes32,bytes32)[])")]
    fn verify_query(
        handle: &mut impl PrecompileHandle,
        query: Query,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<ContinuityBlock>,
    ) -> EvmResult<QueryVerificationResult> {
        // Log the query verification attempt
        handle.record_log_costs_manual(3, 32)?;

        // Base cost for invoking the precompile
        handle.record_cost(GAS_BASE_VERIFY)?;

        // Convert bounded bytes to Vec<u8>
        let tx_bytes: Vec<u8> = tx_data.into();

        // Validate inputs
        if tx_bytes.is_empty() {
            error!(
                "Empty transaction data submitted for query: {:?}",
                query.id()
            );
            let encoded_revert = encode_revert_message("Transaction data cannot be empty");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        // Note: Empty siblings is valid for single-transaction blocks

        // Charge for transaction data processing
        handle.record_cost(GAS_PER_TX_BYTE.saturating_mul(tx_bytes.len() as u64))?;

        // Charge for Merkle proof verification
        handle.record_cost(GAS_PER_SIBLING.saturating_mul(merkle_proof.siblings.len() as u64))?;
        let merkle_weight = sp_weights::Weight::from_parts(WEIGHT_MERKLE_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, merkle_weight, 0)?;

        // Step 1: Verify Merkle proof
        let merkle_valid = Self::verify_merkle_proof(handle, &tx_bytes, &merkle_proof, &query)?;

        if !merkle_valid {
            error!(
                "Merkle proof verification failed for query: {:?}",
                query.id()
            );
            return Ok(QueryVerificationResult {
                status: 1, // MerkleProofInvalid
                result_segments: Vec::new(),
            });
        }

        // Charge for continuity chain verification
        handle
            .record_cost(GAS_PER_CONTINUITY_BLOCK.saturating_mul(continuity_blocks.len() as u64))?;
        let continuity_weight = sp_weights::Weight::from_parts(WEIGHT_CONTINUITY_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, continuity_weight, 0)?;

        // Step 2: Verify continuity chain
        let blocks: Vec<Block> = continuity_blocks.into_iter().map(|cb| cb.into()).collect();
        let continuity_valid = Self::verify_continuity_chain(handle, &blocks, &query)?;

        if !continuity_valid {
            error!(
                "Continuity chain verification failed for query: {:?}",
                query.id()
            );
            return Ok(QueryVerificationResult {
                status: 2, // ContinuityChainInvalid
                result_segments: Vec::new(),
            });
        }

        // Step 3: Extract data segments
        let result_segments = Self::extract_data_segments(&tx_bytes, &query)?;

        Ok(QueryVerificationResult {
            status: 0, // Success
            result_segments,
        })
    }

    /// Verify the Merkle proof for transaction inclusion using Keccak256 hash
    ///
    /// This implements the MMR Merkle tree verification with:
    /// 1. Leaf hashing: prepend 0x00 to tx_data and hash with Keccak256
    /// 2. Inner node hashing: prepend 0x01 to (left + right) and hash with Keccak256
    /// 3. Tree traversal: use query.index to determine sibling positions
    fn verify_merkle_proof(
        _handle: &mut impl PrecompileHandle,
        tx_data: &[u8],
        merkle_proof: &MerkleProof,
        query: &Query,
    ) -> Result<bool, PrecompileFailure> {
        let max_size = 10485760usize; // 10MB
        let tx_data = &tx_data[..tx_data.len().min(max_size)];

        // Step 1: Hash the transaction data as a leaf node
        // Prepend LEAF_HASH_PREPEND_VALUE (0x00) to tx_data before hashing
        let mut prefixed_leaf = sp_std::vec![0u8; tx_data.len() + 1];
        prefixed_leaf[0] = 0x00; // LEAF_HASH_PREPEND_VALUE
        prefixed_leaf[1..].copy_from_slice(tx_data);

        let current_hash = sp_io::hashing::keccak_256(&prefixed_leaf);

        // Step 2: Handle single-transaction case (no siblings)
        if merkle_proof.siblings.is_empty() {
            let computed_root = H256::from(current_hash);
            let result = computed_root == merkle_proof.root;
            return Ok(result);
        }

        // Step 3: Traverse the Merkle tree using siblings
        // Each level has ARITY (2) siblings that represent the complete set of child hashes
        let mut current_hash = H256::from(current_hash);
        let mut index = query.index;
        let arity = 2usize; // Binary tree

        let siblings_per_level = arity; // We have ARITY hashes per level (including placeholder)
        let num_levels = merkle_proof.siblings.len() / siblings_per_level;

        // Process each level of the tree
        // siblings contains ALL hashes per level (ARITY hashes including placeholder at offset)
        for level in 0..num_levels {
            let start = level * siblings_per_level;
            let end = start + siblings_per_level;

            if end > merkle_proof.siblings.len() {
                error!("  Level {level} would exceed siblings array bounds");
                break;
            }

            // Get all hashes for this level (including placeholder at offset)
            let level_siblings = &merkle_proof.siblings[start..end];

            // Determine which position our current hash occupies
            let offset = (index % arity as u64) as usize;

            // Build the hash input with inner node prefix
            let mut hash_input = sp_std::vec![0x01u8]; // INNER_HASH_PREPEND_VALUE

            // Add child hashes in order, replacing placeholder with current_hash
            for (i, sibling) in level_siblings.iter().enumerate().take(arity) {
                if i == offset {
                    // Replace placeholder with our computed hash at this position
                    hash_input.extend_from_slice(&current_hash.0);
                } else {
                    // Use the hash from the proof at this position
                    hash_input.extend_from_slice(&sibling.0);
                }
            }

            // Hash with Keccak256
            current_hash = H256::from(sp_io::hashing::keccak_256(&hash_input));

            // Move up the tree
            index /= arity as u64;
        }

        // Step 4: Compare computed root with provided root
        let result = current_hash == merkle_proof.root;

        Ok(result)
    }

    /// Verify the continuity chain of block attestations
    ///
    /// Verify the continuity chain of block attestations
    fn verify_continuity_chain(
        handle: &mut impl PrecompileHandle,
        continuity_blocks: &[Block],
        query: &Query,
    ) -> Result<bool, PrecompileFailure> {
        // For now, allow empty continuity chain for testing
        if continuity_blocks.is_empty() {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity chain cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Get the last finalized digest for this chain
        // Try last attestation first, then fall back to last checkpoint
        let mut last_finalized_digest = Self::last_digest(query.chain_id)
            .or_else(|| Self::last_checkpoint(query.chain_id).map(|cp| cp.digest))
            .ok_or_else(|| {
                error!(
                    "❌ No finalized attestation or checkpoint found for chain_id {}",
                    query.chain_id
                );
                let encoded_revert =
                    encode_revert_message("No finalized attestation or checkpoint found");
                PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                }
            })?;

        // Validate the tail's prev_digest matches a known attestation
        if let Some(tail) = continuity_blocks.first() {
            let block_prev_digest = tail.prev_digest;

            // Charge for storage lookup
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

            // Check if the tail's prev_digest matches a known attestation or checkpoint
            if let Some(prev_attestation) = Self::get_attestation(query.chain_id, block_prev_digest)
            {
                if prev_attestation.attestation.header_number != tail.block_number - 1 {
                    error!(
                        "❌ Continuity proof tail prev digest points to attestation with header number {}, but expected {}",
                        prev_attestation.attestation.header_number,
                        tail.block_number - 1
                    );
                    return Ok(false);
                }
                // Update last_finalized_digest to the tail's prev_digest
                last_finalized_digest = block_prev_digest;
            } else if let Some(checkpoint_block_number) =
                Self::get_checkpoint(query.chain_id, block_prev_digest)
            {
                // The prev_digest points to a checkpoint
                if checkpoint_block_number != tail.block_number - 1 {
                    error!(
                        "❌ Continuity proof tail prev digest points to checkpoint with block number {}, but expected {}",
                        checkpoint_block_number,
                        tail.block_number - 1
                    );
                    return Ok(false);
                }
                // Update last_finalized_digest to the tail's prev_digest
                last_finalized_digest = block_prev_digest;
            } else {
                error!(
                    "❌ Continuity proof tail prev digest {block_prev_digest:?} not found in attestations or checkpoints"
                );
                return Ok(false);
            }
        }

        // Validate each block in the continuity chain
        for cb in continuity_blocks {
            let block_digest = cb.digest;
            let block_prev_digest = cb.prev_digest;

            // Verify the link continues exactly
            if last_finalized_digest != block_prev_digest {
                error!(
                    "❌ Continuity proof break: expected prev_digest {last_finalized_digest:?}, got {block_prev_digest:?}"
                );
                return Ok(false);
            }
            // Charge for the block verification
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

            // Update the last block digest to the current block's digest
            last_finalized_digest = block_digest;
        }

        // Validate the head's digest matches the query's previous requirement
        if let Some(head) = continuity_blocks.last() {
            let _block_digest = head.digest;

            // The head should connect to the query height
            // For now, we just validate that the continuity chain is internally consistent
            // Additional query-specific validation can be added here
        }

        // info!(
        //     "Continuity chain verified successfully for query: {:?}",
        //     query.id()
        // );

        Ok(true)
    }

    /// Extract data segments from transaction data according to query layout
    ///
    /// For each layout segment:
    /// 1. Validate offset + size is within tx_data bounds
    /// 2. Extract bytes from tx_data at specified offset
    /// 3. Convert to H256 (right-aligned, left-padded with zeros if < 32 bytes)
    fn extract_data_segments(
        tx_data: &[u8],
        query: &Query,
    ) -> Result<Vec<ResultSegment>, PrecompileFailure> {
        let mut result_segments = Vec::new();

        for segment in &query.layout_segments {
            let start = segment.offset as usize;
            let end = start + segment.size as usize;

            // Validate bounds
            if end > tx_data.len() {
                error!(
                    "Layout segment out of bounds: offset={}, size={}, tx_data_len={}",
                    segment.offset,
                    segment.size,
                    tx_data.len()
                );
                let encoded_revert =
                    encode_revert_message("Data extraction error: segment out of bounds");
                return Err(PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                });
            }

            // Extract the byte slice
            let extracted_bytes = &tx_data[start..end];

            // Convert to H256 based on size
            let result_bytes = if segment.size == 32 {
                // Exact 32 bytes - direct conversion
                H256::from_slice(extracted_bytes)
            } else if segment.size < 32 {
                // Less than 32 bytes - right-align (big-endian style)
                // Pad left with zeros
                let mut padded = [0u8; 32];
                let offset = 32 - extracted_bytes.len();
                padded[offset..].copy_from_slice(extracted_bytes);
                H256::from(padded)
            } else {
                // More than 32 bytes - take first 32 bytes
                // This handles cases where segments might be larger than H256
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&extracted_bytes[..32]);
                H256::from(bytes)
            };

            result_segments.push(ResultSegment {
                offset: segment.offset,
                bytes: result_bytes,
            });
        }

        Ok(result_segments)
    }
}
