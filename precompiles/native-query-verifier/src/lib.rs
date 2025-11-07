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
use fp_evm::{ExitError, ExitRevert, PrecompileFailure, PrecompileHandle};
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
type ConstU10MB = sp_core::ConstU32<10_485_760>; // Type alias for bounded vec

// Batch queries constraint
type MaxBatchSize = sp_core::ConstU32<10>;

/// Result of query verification
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct QueryVerificationResult {
    /// Verification status: 0 = Success, 1 = MerkleProofInvalid, 2 = ContinuityChainInvalid, 3 = DataExtractionError, 4 = MerkleRootMismatch
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

    /// Verify a single query against its merkle proof and continuity chain
    /// This is the core verification logic used by both single and batch verification
    /// Note: Gas charging is handled by the caller
    fn verify_single_query_internal(
        _handle: &mut impl PrecompileHandle,
        query: &Query,
        tx_bytes: &[u8],
        merkle_proof: &MerkleProof,
        continuity_blocks: &[Block],
    ) -> EvmResult<QueryVerificationResult> {
        // Step 1: Verify Merkle proof
        let merkle_valid = merkle_proof.verify(tx_bytes);

        if !merkle_valid {
            return Ok(QueryVerificationResult {
                status: 1, // MerkleProofInvalid
                result_segments: Vec::new(),
            });
        }

        // Step 2: Check if continuity chain contains the query block
        // If not, we can't verify the merkle root against the continuity chain
        let query_block = continuity_blocks
            .iter()
            .find(|b| b.block_number == query.height);

        // Step 3: If we have the query block, verify merkle root matches
        if let Some(query_block) = query_block {
            if merkle_proof.root != query_block.root {
                error!(
                    "Merkle root mismatch: proof root {:?} != block root {:?}",
                    merkle_proof.root, query_block.root
                );
                return Ok(QueryVerificationResult {
                    status: 4, // MerkleRootMismatch
                    result_segments: Vec::new(),
                });
            }
        } else {
            // Query block not in continuity chain - this is a continuity chain issue
            error!("Query block {} not found in continuity chain", query.height);
            return Ok(QueryVerificationResult {
                status: 2, // ContinuityChainInvalid
                result_segments: Vec::new(),
            });
        }

        // Step 4: Extract data segments
        match Self::extract_data_segments(tx_bytes, query) {
            Ok(segments) => Ok(QueryVerificationResult {
                status: 0, // Success
                result_segments: segments,
            }),
            Err(e @ PrecompileFailure::Revert { .. }) => {
                // Propagate revert errors (like segment out of bounds)
                Err(e)
            }
            Err(e) => {
                // Other extraction errors return status 3
                error!("Failed to extract data segments: {e:?}");
                Ok(QueryVerificationResult {
                    status: 3, // DataExtractionError
                    result_segments: Vec::new(),
                })
            }
        }
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
    #[precompile::public("verifyQuery((uint64,uint64,(uint64,uint64)[]),bytes,(bytes32,(bytes32,bool)[]),(uint64,bytes32,bytes32,bytes32)[])")]
    fn verify_query(
        handle: &mut impl PrecompileHandle,
        query: Query,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<Block>,
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

        // Check for empty continuity chain
        if continuity_blocks.is_empty() {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity chain cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Charge for continuity chain verification
        let continuity_gas = GAS_PER_CONTINUITY_BLOCK
            .checked_mul(continuity_blocks.len() as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(continuity_gas)?;
        let continuity_weight = sp_weights::Weight::from_parts(WEIGHT_CONTINUITY_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, continuity_weight, 0)?;

        // Verify continuity chain
        let blocks: Vec<Block> = continuity_blocks;
        let continuity_valid = Self::verify_continuity_chain(handle, &blocks, &query)?;

        if !continuity_valid {
            return Self::handle_verification_failure(
                handle,
                &query,
                2, // ContinuityChainInvalid
                "Continuity chain validation failed",
            );
        }

        // Charge for transaction data processing
        let tx_gas =
            GAS_PER_TX_BYTE
                .checked_mul(tx_bytes.len() as u64)
                .ok_or(PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                })?;
        handle.record_cost(tx_gas)?;

        // Charge for Merkle proof verification
        let merkle_gas = GAS_PER_SIBLING
            .checked_mul(merkle_proof.siblings.len() as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(merkle_gas)?;
        let merkle_weight = sp_weights::Weight::from_parts(WEIGHT_MERKLE_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, merkle_weight, 0)?;

        // Use the common verification logic
        let result =
            Self::verify_single_query_internal(handle, &query, &tx_bytes, &merkle_proof, &blocks)?;

        // Handle the result
        if result.status == 0 {
            // Emit success event
            Self::emit_verification_success(handle, &query, &result.result_segments)?;
        } else {
            // Map status codes to error messages
            let message = match result.status {
                1 => "Merkle proof validation failed",
                2 => "Query block not found in continuity chain",
                3 => "Failed to extract data segments",
                4 => "Merkle root mismatch",
                _ => "Unknown verification error",
            };
            return Self::handle_verification_failure(handle, &query, result.status, message);
        }

        Ok(result)
    }

    /// Verify a batch of queries with shared continuity proof
    ///
    /// This function verifies multiple queries that share a common continuity chain,
    /// optimizing gas costs by verifying the continuity proof only once.
    /// Verify batch queries with a shared continuity chain (POC-optimized)
    ///
    /// Uses relaxed verification mode for efficiency:
    /// 1. Verifies the continuity proof chain once (shared across all queries)
    /// 2. For each query, only verifies:
    ///    - Merkle proof for transaction inclusion
    ///    - Query block exists in continuity chain at correct position
    ///    - Merkle root matches the continuity block
    ///
    /// This is more efficient than strict verification as it avoids redundant
    /// continuity chain validation for each query. The primary goal is proving
    /// inclusion - as long as the transaction is in the block and the block's
    /// merkle root is validated against a checkpoint, full sequential chain
    /// verification per query is not necessary.
    ///
    /// # Arguments
    /// * `queries` - Vector of queries to verify
    /// * `tx_data_array` - Transaction data for each query
    /// * `merkle_proofs` - Merkle proofs for each query
    /// * `shared_continuity_blocks` - Shared continuity chain covering all queries
    ///
    /// # Returns
    /// `BatchQueryVerificationResult` with statistics and individual results
    #[precompile::public("verifyBatchQueries((uint64,uint64,(uint64,uint64)[])[],bytes[],(bytes32,(bytes32,bool)[])[],(uint64,bytes32,bytes32,bytes32)[])")]
    fn verify_batch_queries(
        handle: &mut impl PrecompileHandle,
        queries: BoundedVec<Query, MaxBatchSize>,
        tx_data_array: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<MerkleProof>,
        shared_continuity_blocks: Vec<Block>,
    ) -> EvmResult<BatchQueryVerificationResult> {
        let queries: Vec<Query> = queries.into();
        let num_queries = queries.len();
        log::info!("Processing batch of {num_queries} queries");

        // Validate input arrays have same length
        if num_queries != tx_data_array.len() || num_queries != merkle_proofs.len() {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Input arrays must have the same length"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Calculate gas for batch operation
        let base_gas_per_query = GAS_BASE_VERIFY;
        let total_base_gas =
            base_gas_per_query
                .checked_mul(num_queries as u64)
                .ok_or(PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                })?;
        handle.record_cost(total_base_gas)?;

        // Find min and max block heights from all queries
        let min_height =
            queries
                .iter()
                .map(|q| q.height)
                .min()
                .ok_or_else(|| PrecompileFailure::Revert {
                    output: encode_revert_message("Empty queries array"),
                    exit_status: ExitRevert::Reverted,
                })?;

        let max_height = queries.iter().map(|q| q.height).max().unwrap(); // Safe because we already checked for non-empty

        // Verify shared continuity chain once (more efficient than verifying per query)
        if !shared_continuity_blocks.is_empty() {
            let continuity_gas = GAS_PER_CONTINUITY_BLOCK
                .checked_mul(shared_continuity_blocks.len() as u64)
                .ok_or(PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                })?;
            handle.record_cost(continuity_gas)?;

            // Verify continuity chain covers the range of all queries
            if let Some(first_block) = shared_continuity_blocks.first() {
                if first_block.block_number > min_height {
                    return Err(PrecompileFailure::Revert {
                        output: encode_revert_message(
                            "Continuity chain doesn't cover minimum query height",
                        ),
                        exit_status: ExitRevert::Reverted,
                    });
                }
            }

            if let Some(last_block) = shared_continuity_blocks.last() {
                if last_block.block_number < max_height {
                    return Err(PrecompileFailure::Revert {
                        output: encode_revert_message(
                            "Continuity chain doesn't cover maximum query height",
                        ),
                        exit_status: ExitRevert::Reverted,
                    });
                }
            }

            // Verify the continuity chain itself (using first query for chain_id)
            let first_query = &queries[0];
            let continuity_valid =
                Self::verify_continuity_chain(handle, &shared_continuity_blocks, first_query)?;

            if !continuity_valid {
                // If continuity fails, revert the entire batch
                return Err(PrecompileFailure::Revert {
                    output: encode_revert_message("Continuity chain validation failed for batch"),
                    exit_status: ExitRevert::Reverted,
                });
            }
        }

        // Process each query
        let mut results = Vec::with_capacity(num_queries);
        let mut successful_queries = 0u32;
        let mut failed_queries = 0u32;

        for (i, ((query, tx_data), merkle_proof)) in queries
            .into_iter()
            .zip(tx_data_array.into_iter())
            .zip(merkle_proofs.into_iter())
            .enumerate()
        {
            log::info!("Processing query {}/{}", i + 1, num_queries);

            // Convert bounded bytes to Vec<u8>
            let tx_bytes: Vec<u8> = tx_data.into();

            // Charge per-transaction data gas
            let data_gas = GAS_PER_TX_BYTE.checked_mul(tx_bytes.len() as u64).ok_or(
                PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                },
            )?;
            handle.record_cost(data_gas)?;

            // Verify Merkle proof
            let merkle_gas = GAS_PER_SIBLING
                .checked_mul(merkle_proof.siblings.len() as u64)
                .ok_or(PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                })?;
            handle.record_cost(merkle_gas)?;

            // POC-style relaxed verification for batch queries:
            // 1. Verify Merkle proof for transaction inclusion
            let merkle_valid = merkle_proof.verify(&tx_bytes);

            if !merkle_valid {
                // Merkle proof failed - record failure but don't emit event
                let result = QueryVerificationResult {
                    status: 1, // MerkleProofInvalid
                    result_segments: Vec::new(),
                };
                failed_queries += 1;
                results.push(result);
                continue;
            }

            // 2. Find the query block in the continuity chain (relaxed check)
            // POC optimization: We can compute the index from the query height
            // if blocks are sequential. First, check if we have a valid range.
            let first_block_num = shared_continuity_blocks
                .first()
                .map(|b| b.block_number)
                .unwrap_or(0);
            let last_block_num = shared_continuity_blocks
                .last()
                .map(|b| b.block_number)
                .unwrap_or(0);

            // Try to find by computed index first (more efficient)
            let query_block = if query.height >= first_block_num && query.height <= last_block_num {
                // Compute index: query.height - first_block_number
                let index = (query.height - first_block_num) as usize;
                if index < shared_continuity_blocks.len() {
                    let block = &shared_continuity_blocks[index];
                    // Verify the computed index is correct
                    if block.block_number == query.height {
                        Some(block)
                    } else {
                        // Fall back to linear search if blocks aren't sequential
                        shared_continuity_blocks
                            .iter()
                            .find(|b| b.block_number == query.height)
                    }
                } else {
                    None
                }
            } else {
                // Query height is outside the range
                None
            };

            if let Some(query_block) = query_block {
                // 3. Verify merkle root matches the continuity block
                if merkle_proof.root != query_block.root {
                    // Root mismatch
                    let result = QueryVerificationResult {
                        status: 4, // MerkleRootMismatch
                        result_segments: Vec::new(),
                    };
                    // Root mismatch - record failure but don't emit event
                    failed_queries += 1;
                    results.push(result);
                    continue;
                }

                // 4. Extract data segments (only if merkle proof and root match succeeded)
                match Self::extract_data_segments(&tx_bytes, &query) {
                    Ok(segments) => {
                        let result = QueryVerificationResult {
                            status: 0, // Success
                            result_segments: segments.clone(),
                        };
                        Self::emit_verification_success(handle, &query, &segments)?;
                        successful_queries += 1;
                        results.push(result);
                    }
                    Err(e @ PrecompileFailure::Revert { .. }) => {
                        // Propagate revert errors
                        return Err(e);
                    }
                    Err(_) => {
                        // Other extraction errors - record failure but don't emit event
                        let result = QueryVerificationResult {
                            status: 3, // DataExtractionError
                            result_segments: Vec::new(),
                        };
                        failed_queries += 1;
                        results.push(result);
                    }
                }
            } else {
                // Query block not found in continuity chain - record failure but don't emit event
                let result = QueryVerificationResult {
                    status: 2, // ContinuityChainInvalid
                    result_segments: Vec::new(),
                };
                failed_queries += 1;
                results.push(result);
            }
        }

        // Emit batch summary event (in addition to individual events)
        log::info!(
            "Batch verification completed: {successful_queries} successful, {failed_queries} failed"
        );

        let event_data = ethabi::encode(&[
            Token::Uint(successful_queries.into()),
            Token::Uint(failed_queries.into()),
            Token::Uint((successful_queries + failed_queries).into()),
        ]);

        log2(
            handle.context().address,
            SELECTOR_LOG_BATCH_QUERIES_VERIFIED,
            handle.context().caller,
            event_data,
        )
        .record(handle)?;

        Ok(BatchQueryVerificationResult {
            successful_queries,
            failed_queries,
            results,
        })
    }

    /// Generic handler for verification failures
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

    /// Verify the continuity chain of block attestations
    ///
    /// Note: For POC optimization compatibility, batch verification uses
    /// implicit block numbering where block_number can be computed from index
    /// (block_number = start_block + index). This optimization is implemented
    /// directly in the batch verification logic for efficiency.
    fn verify_continuity_chain(
        handle: &mut impl PrecompileHandle,
        continuity_blocks: &[Block],
        query: &Query,
    ) -> Result<bool, PrecompileFailure> {
        // Should not be called with empty continuity_blocks, but check anyway
        if continuity_blocks.is_empty() {
            return Ok(false);
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

        // Validate the continuity chain reaches the query height
        if let Some(head) = continuity_blocks.last() {
            if head.block_number < query.height {
                error!(
                    "❌ Continuity chain ends at block {}, but query requires block {}",
                    head.block_number, query.height
                );
                return Err(PrecompileFailure::Revert {
                    output: encode_revert_message("Continuity chain does not reach query height"),
                    exit_status: ExitRevert::Reverted,
                });
            }
        }

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
