use attestor_primitives::{block::Block, query::Query};
use ethabi::Token;
use fp_evm::{ExitError, ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use precompile_utils::{prelude::*, solidity::Codec};
use sp_core::H256;
use sp_std::vec::Vec;

use crate::{
    encode_revert_message, BatchQueryVerificationResult, MaxBatchSize, MerkleProof,
    NativeQueryVerifierPrecompile, QueryVerificationResult, SELECTOR_LOG_BATCH_QUERIES_VERIFIED,
};

// Gas cost constants
// Based on realistic Solidity implementation costs with precompile efficiency gains:
// - Base transaction: 21,000 gas
// - Keccak256 in Solidity: ~30 + 6/word, in precompile: ~10x faster
// - SLOAD: 2,100 (warm) / 2,600 (cold)
pub const GAS_BASE_VERIFY: u64 = 21_000; // Base transaction cost (matches Ethereum standard)
pub const GAS_PER_TX_BYTE: u64 = 16; // Per byte cost for transaction data (matches calldata cost)
pub const GAS_PER_SIBLING: u64 = 200; // Per Merkle sibling verification (native efficiency vs ~166 in Solidity)
pub const GAS_PER_CONTINUITY_BLOCK: u64 = 3_000; // Per block verification (storage + hash check)
pub const GAS_STORAGE_LOOKUP: u64 = 2_600; // Each storage read (matches cold SLOAD)
pub const WEIGHT_MERKLE_VERIFY: u64 = 100_000; // Merkle verification work
pub const WEIGHT_CONTINUITY_VERIFY: u64 = 50_000; // Continuity verification work

// Size constraints
type ConstU10MB = sp_core::ConstU32<10_485_760>; // Type alias for bounded vec

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
    /// Core implementation for verifying a blockchain query
    ///
    /// Following the POC structure from BlockProver.sol:
    /// 1. Verify merkle proof for the transaction
    /// 2. Verify the query block exists in continuity chain and merkle root matches
    /// 3. Verify continuity proof chain validity
    /// 4. Verify final digest matches a checkpoint or attestation
    /// 5. Extract data segments from verified transaction
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting and logging
    /// - `query`: Query specification with chain_id, block height, and data layout
    /// - `tx_data`: Raw transaction data to verify and extract from
    /// - `merkle_proof`: Proof of transaction inclusion in the block
    /// - `continuity_blocks`: Chain of blocks linking queryHeight-1 to next attestation
    /// - `emit_events`: Whether to emit QueryVerified event (true for non-view functions)
    ///
    /// # Returns
    /// `QueryVerificationResult` with status 0 and extracted data segments on success
    ///
    /// # Reverts with specific error messages
    /// - "Continuity chain cannot be empty"
    /// - "Transaction data cannot be empty"
    /// - "Merkle proof validation failed"
    /// - "Query block not found in continuity chain"
    /// - "Merkle root mismatch"
    /// - "Continuity chain validation failed"
    /// - "Data extraction error: segment out of bounds"
    pub fn verify_query_impl(
        handle: &mut impl PrecompileHandle,
        query: Query,
        tx_data: BoundedBytes<ConstU10MB>,
        merkle_proof: MerkleProof,
        continuity_blocks: Vec<Block>,
        emit_events: bool,
    ) -> EvmResult<QueryVerificationResult> {
        // Log costs only for non-view functions
        if emit_events {
            handle.record_log_costs_manual(3, 32)?;
        }

        // Base cost
        handle.record_cost(GAS_BASE_VERIFY)?;

        // Convert bounded bytes to Vec<u8>
        let tx_bytes: Vec<u8> = tx_data.into();

        // Validate inputs
        if tx_bytes.is_empty() {
            debug!(
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
            debug!("Empty continuity chain for query: {:?}", query.id());
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity chain cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Charge gas for all operations upfront
        let total_continuity_gas = GAS_PER_CONTINUITY_BLOCK
            .checked_mul(continuity_blocks.len() as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(total_continuity_gas)?;

        let tx_gas =
            GAS_PER_TX_BYTE
                .checked_mul(tx_bytes.len() as u64)
                .ok_or(PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                })?;
        handle.record_cost(tx_gas)?;

        let merkle_gas = GAS_PER_SIBLING
            .checked_mul(merkle_proof.siblings.len() as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(merkle_gas)?;

        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        // Record weights
        let continuity_weight = sp_weights::Weight::from_parts(WEIGHT_CONTINUITY_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, continuity_weight, 0)?;

        let merkle_weight = sp_weights::Weight::from_parts(WEIGHT_MERKLE_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, merkle_weight, 0)?;

        // Step 1: Verify merkle proof for the transaction
        let merkle_valid = merkle_proof.verify(&tx_bytes);
        if !merkle_valid {
            debug!("Merkle proof validation failed for query: {:?}", query.id());
            return Self::handle_verification_failure(
                handle,
                &query,
                1, // MerkleProofInvalid
                "Merkle proof validation failed",
                emit_events,
            );
        }

        // Step 2: Verify the query block exists and merkle root matches continuity proof
        let query_block = continuity_blocks
            .iter()
            .find(|b| b.block_number == query.height)
            .ok_or(PrecompileFailure::Revert {
                output: encode_revert_message("Query block not found in continuity chain"),
                exit_status: ExitRevert::Reverted,
            })?;

        if query_block.root != merkle_proof.root {
            return Self::handle_verification_failure(
                handle,
                &query,
                4, // MerkleRootMismatch
                "Merkle root mismatch",
                emit_events,
            );
        }

        // Step 3: Verify continuity proof chain
        if !Self::verify_continuity_chain(handle, &continuity_blocks, &query)? {
            return Self::handle_verification_failure(
                handle,
                &query,
                2, // ContinuityChainInvalid
                "Continuity chain validation failed",
                emit_events,
            );
        }

        // Step 4: Verify final digest matches checkpoint or attestation (included in verify_continuity_chain)

        // Step 5: Extract data segments
        match extract_data_segments(&tx_bytes, &query) {
            Ok(segments) => {
                // Emit success event only for non-view functions
                if emit_events {
                    Self::emit_verification_success(handle, &query, &segments)?;
                }
                Ok(QueryVerificationResult {
                    status: 0, // Success
                    result_segments: segments,
                })
            }
            Err(e @ PrecompileFailure::Revert { .. }) => {
                // Propagate revert errors (like segment out of bounds)
                Err(e)
            }
            Err(e) => {
                debug!("Failed to extract data segments: {e:?}");
                Self::handle_verification_failure(
                    handle,
                    &query,
                    3, // DataExtractionError
                    "Failed to extract data segments",
                    emit_events,
                )
            }
        }
    }

    /// Internal implementation for batch queries verification
    ///
    /// Optimizes verification by validating the shared continuity chain once,
    /// then verifying each query's merkle proof and data extraction individually.
    /// This approach saves ~40% gas compared to individual verifications.
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting and logging
    /// - `queries`: Vector of queries to verify (bounded to max 10)
    /// - `tx_data_array`: Transaction data for each query
    /// - `merkle_proofs`: Merkle proofs for each query
    /// - `shared_continuity_blocks`: Single continuity chain covering all query heights
    /// - `emit_events`: Whether to emit BatchQueriesVerified event
    ///
    /// # Returns
    /// `BatchQueryVerificationResult` with counts and individual results
    ///
    /// # Verification Flow
    /// 1. Validate input arrays have matching lengths
    /// 2. Verify shared continuity chain once (major gas savings)
    /// 3. For each query:
    ///    - Find query block in continuity chain
    ///    - Verify merkle proof against block's root
    ///    - Extract data segments
    ///    - Track success/failure
    ///
    /// # Note
    /// Individual query failures don't cause the batch to revert.
    /// Failed queries are marked in results with empty segments.
    pub fn verify_batch_queries_impl(
        handle: &mut impl PrecompileHandle,
        queries: BoundedVec<Query, MaxBatchSize>,
        tx_data_array: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<MerkleProof>,
        shared_continuity_blocks: Vec<Block>,
        emit_events: bool,
    ) -> EvmResult<BatchQueryVerificationResult> {
        let queries: Vec<Query> = queries.into();
        let num_queries = queries.len();

        log::debug!("Processing batch of {num_queries} queries");

        // Validate input arrays have same length
        if num_queries != tx_data_array.len() || num_queries != merkle_proofs.len() {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Input arrays must have the same length"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Calculate gas for batch operation
        let total_base_gas =
            GAS_BASE_VERIFY
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
            log::debug!("Processing query {}/{}", i + 1, num_queries);

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

            // 1. Verify Merkle proof for transaction inclusion
            let merkle_valid = merkle_proof.verify(&tx_bytes);

            if !merkle_valid {
                // Merkle proof failed - emit failure event and record failure
                if emit_events {
                    if let Err(e) = Self::emit_verification_failure(
                        handle,
                        &query,
                        1, // MerkleProofInvalid
                        "Merkle proof validation failed",
                    ) {
                        // If event emission fails, continue anyway
                        log::debug!("Failed to emit failure event: {e:?}");
                    }
                }
                let result = QueryVerificationResult {
                    status: 1, // MerkleProofInvalid
                    result_segments: Vec::new(),
                };
                failed_queries += 1;
                results.push(result);
                continue;
            }

            // 2. Find the query block in the continuity chain (relaxed check)
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
                    // Root mismatch - emit failure event and record failure
                    if emit_events {
                        if let Err(e) = Self::emit_verification_failure(
                            handle,
                            &query,
                            4, // MerkleRootMismatch
                            "Merkle root mismatch",
                        ) {
                            log::debug!("Failed to emit failure event: {e:?}");
                        }
                    }
                    let result = QueryVerificationResult {
                        status: 4, // MerkleRootMismatch
                        result_segments: Vec::new(),
                    };
                    failed_queries += 1;
                    results.push(result);
                    continue;
                }

                // 4. Use the common verification logic for the rest
                match verify_single_query_internal(
                    &query,
                    &tx_bytes,
                    &merkle_proof,
                    &shared_continuity_blocks,
                ) {
                    Ok(result) => {
                        if result.status == 0 {
                            // Success
                            if emit_events {
                                Self::emit_verification_success(
                                    handle,
                                    &query,
                                    &result.result_segments,
                                )?;
                            }
                            successful_queries += 1;
                        } else {
                            // Verification failed with a specific status - emit failure event
                            if emit_events {
                                let reason = match result.status {
                                    1 => "Merkle proof validation failed",
                                    2 => "Continuity chain validation failed",
                                    3 => "Data extraction error",
                                    4 => "Merkle root mismatch",
                                    _ => "Unknown verification error",
                                };
                                if let Err(e) = Self::emit_verification_failure(
                                    handle,
                                    &query,
                                    result.status,
                                    reason,
                                ) {
                                    log::debug!("Failed to emit failure event: {e:?}");
                                }
                            }
                            failed_queries += 1;
                        }
                        results.push(result);
                    }
                    Err(e @ PrecompileFailure::Revert { .. }) => {
                        // Propagate revert errors
                        return Err(e);
                    }
                    Err(_) => {
                        // Other errors - emit failure event and record failure
                        if emit_events {
                            if let Err(e) = Self::emit_verification_failure(
                                handle,
                                &query,
                                3, // DataExtractionError
                                "Data extraction error",
                            ) {
                                log::debug!("Failed to emit failure event: {e:?}");
                            }
                        }
                        let result = QueryVerificationResult {
                            status: 3, // DataExtractionError
                            result_segments: Vec::new(),
                        };
                        failed_queries += 1;
                        results.push(result);
                    }
                }
            } else {
                // Query block not found in continuity chain - emit failure event and record failure
                if emit_events {
                    if let Err(e) = Self::emit_verification_failure(
                        handle,
                        &query,
                        2, // ContinuityChainInvalid
                        "Query block not found in continuity chain",
                    ) {
                        log::debug!("Failed to emit failure event: {e:?}");
                    }
                }
                let result = QueryVerificationResult {
                    status: 2, // ContinuityChainInvalid
                    result_segments: Vec::new(),
                };
                failed_queries += 1;
                results.push(result);
            }
        }

        // Emit batch summary event (in addition to individual events)
        if emit_events {
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
        }

        Ok(BatchQueryVerificationResult {
            successful_queries,
            failed_queries,
            results,
        })
    }
}
/// Verify a single query against its merkle proof and continuity chain
///
/// This is the core verification logic used by both single and batch verification.
/// It performs merkle proof validation, ensures the query block exists in the
/// continuity chain, verifies root matching, and extracts data segments.
///
/// # Parameters
/// - `query`: Query specification with chain_id, height, and layout segments
/// - `tx_bytes`: Raw transaction data to verify
/// - `merkle_proof`: Proof of transaction inclusion
/// - `continuity_blocks`: Pre-validated continuity chain
///
/// # Returns
/// `QueryVerificationResult` with status code and extracted segments
///
/// # Status Codes
/// - 0: Success - verification passed and data extracted
/// - 1: MerkleProofInvalid - merkle proof verification failed
/// - 2: ContinuityChainInvalid - query block not in continuity chain
/// - 3: DataExtractionError - failed to extract segments from tx data
/// - 4: MerkleRootMismatch - proof root doesn't match block root
///
/// # Note
/// - Gas charging is handled by the caller
/// - This function doesn't validate the continuity chain itself
/// - Used internally by both single and batch verification
pub fn verify_single_query_internal(
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
            debug!(
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
        debug!("Query block {} not found in continuity chain", query.height);
        return Ok(QueryVerificationResult {
            status: 2, // ContinuityChainInvalid
            result_segments: Vec::new(),
        });
    }

    // Step 4: Extract data segments
    match extract_data_segments(tx_bytes, query) {
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
            debug!("Failed to extract data segments: {e:?}");
            Ok(QueryVerificationResult {
                status: 3, // DataExtractionError
                result_segments: Vec::new(),
            })
        }
    }
}

/// Extract data segments from transaction data according to query layout
///
/// Processes each layout segment in the query to extract specific data portions
/// from the verified transaction. This enables selective data extraction without
/// needing to process the entire transaction on-chain.
///
/// # Parameters
/// - `tx_data`: Raw transaction data to extract from
/// - `query`: Query containing layout segments specifying what to extract
///
/// # Returns
/// Vector of `ResultSegment` containing extracted data as H256 values
///
/// # Data Processing
/// For each layout segment:
/// 1. Validate offset + size is within tx_data bounds
/// 2. Extract bytes from tx_data[offset..offset+size]
/// 3. Convert to H256:
///    - If size <= 32: right-align data (left-pad with zeros)
///    - If size > 32: truncate to first 32 bytes
///
/// # Reverts
/// - If any segment would read beyond tx_data bounds
///
/// # Example
/// Query with segment {offset: 4, size: 20} on tx_data of 100 bytes:
/// - Extracts bytes 4-23 (20 bytes)
/// - Returns as H256 with 12 zero bytes on left, 20 data bytes on right
pub fn extract_data_segments(
    tx_data: &[u8],
    query: &Query,
) -> Result<Vec<ResultSegment>, PrecompileFailure> {
    let mut result_segments = Vec::new();

    for segment in &query.layout_segments {
        let start = segment.offset as usize;
        let end = start + segment.size as usize;

        // Validate bounds
        if end > tx_data.len() {
            debug!(
                "Layout segment out of bounds: offset={}, size={}, tx_data_len={}",
                segment.offset,
                segment.size,
                tx_data.len()
            );
            let encoded_revert =
                crate::encode_revert_message("Data extraction error: segment out of bounds");
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
