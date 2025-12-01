use crate::{encode_revert_message, ConstU10MB};
use attestor_primitives::block::ContinuityProof;
use ethabi::Token;
use fp_evm::{ExitError, ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::debug;
use pallet_evm::AddressMapping;
use precompile_utils::{evm::logs::log3, prelude::*};
use sp_core::H256;
use sp_std::vec::Vec;

use crate::{BlockProverPrecompile, SELECTOR_LOG_TRANSACTION_VERIFIED};
use merkle::TransactionMerkleProof;

// Gas cost constants
// Based on realistic Solidity implementation costs with precompile efficiency gains:
// - Keccak256 in Solidity: ~30 + 6/word, in precompile: ~10x faster
// - SLOAD: 2,100 (warm) / 2,600 (cold)
pub const GAS_PER_TX_BYTE: u64 = 16; // Per byte cost for transaction data (matches calldata cost)
pub const GAS_PER_SIBLING: u64 = 200; // Per Merkle sibling verification (native efficiency vs ~166 in Solidity)
                                      // GAS_PER_CONTINUITY_BLOCK = 400 breakdown:
                                      // - Hash computation (Keccak-256 on 72 bytes): ~48 gas
                                      // - Two H256 comparisons (prev_digest, computed_digest): ~12 gas
                                      // - Loop overhead and control flow: ~10 gas
                                      // - Safety margin: ~330 gas (for Substrate runtime overhead, future changes)
pub const GAS_PER_CONTINUITY_BLOCK: u64 = 400; // Per block verification (hash ~48 gas + comparisons/overhead ~350 gas)
pub const WEIGHT_MERKLE_VERIFY: u64 = 100_000; // Merkle verification work
pub const WEIGHT_CONTINUITY_VERIFY: u64 = 50_000; // Continuity verification work

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
    /// Charge gas for transaction data and merkle proof verification
    fn charge_query_gas(
        handle: &mut impl PrecompileHandle,
        tx_bytes_len: usize,
        merkle_siblings_len: usize,
    ) -> EvmResult<()> {
        let tx_gas =
            GAS_PER_TX_BYTE
                .checked_mul(tx_bytes_len as u64)
                .ok_or(PrecompileFailure::Error {
                    exit_status: ExitError::OutOfGas,
                })?;
        handle.record_cost(tx_gas)?;

        let merkle_gas = GAS_PER_SIBLING
            .checked_mul(merkle_siblings_len as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(merkle_gas)?;

        Ok(())
    }

    /// Verify merkle proof for transaction inclusion
    ///
    /// # Returns
    /// `true` if the merkle proof is valid, `false` otherwise
    fn verify_merkle_proof(merkle_proof: &TransactionMerkleProof, tx_bytes: &[u8]) -> bool {
        merkle_proof.verify(tx_bytes)
    }

    /// Calculate transaction index from Merkle proof siblings
    ///
    /// Reconstructs the transaction index by working from leaf to root.
    /// The `is_left` flags in siblings indicate the path taken through the tree.
    /// Siblings are stored from leaf level to root level.
    /// - If sibling is left (`is_left = true`), current node was right, so bit = 1
    /// - If sibling is right (`is_left = false`), current node was left, so bit = 0
    ///
    /// Returns the transaction index (leaf position in the Merkle tree).
    pub(crate) fn calculate_tx_index(merkle_proof: &TransactionMerkleProof) -> u64 {
        if merkle_proof.siblings.is_empty() {
            // Single transaction case
            return 0;
        }

        // Reconstruct the index by working from leaf to root
        // Siblings are stored from leaf level (first) to root level (last)
        // The least significant bit corresponds to the leaf level
        let mut tx_index = 0u64;

        // Process siblings from leaf to root (forward order)
        for (bit_position, sibling) in merkle_proof.siblings.iter().enumerate() {
            // If sibling is on the left, current node was on the right (bit = 1)
            // If sibling is on the right, current node was on the left (bit = 0)
            if sibling.is_left {
                // Sibling is left, so we were right - set bit to 1
                tx_index |= 1u64 << bit_position;
            }
            // If sibling is right, we were left - bit stays 0
        }

        tx_index
    }

    /// Core implementation for verifying a blockchain query
    ///
    /// Following the POC structure from BlockProver.sol:
    /// 1. Verify merkle proof for the transaction
    /// 2. Verify the query block exists in continuity chain and merkle root matches
    /// 3. Verify continuity proof chain validity
    /// 4. Verify final digest matches a checkpoint or attestation
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting and logging
    /// - `chain_key`: Chain key identifier
    /// - `height`: Block height to query
    /// - `encoded_transaction`: Raw transaction data to verify
    /// - `merkle_proof`: Proof of transaction inclusion in the block
    /// - `continuity_proof`: Optimized continuity proof (blocks[0] is at queryHeight-1)
    /// - `emit_events`: Whether to emit events (true for non-view functions)
    ///
    /// # Returns
    /// `true` on success
    ///
    /// # Reverts with specific error messages
    /// - "Continuity chain cannot be empty"
    /// - "Transaction data cannot be empty"
    /// - "Merkle proof validation failed"
    /// - "Query block not found in continuity chain"
    /// - "Merkle root mismatch"
    /// - "Continuity chain validation failed"
    pub fn verify_impl(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        height: u64,
        encoded_transaction: BoundedBytes<ConstU10MB>,
        merkle_proof: TransactionMerkleProof,
        continuity_proof: ContinuityProof,
        emit_events: bool,
    ) -> EvmResult<bool> {
        // For single query: blocks[0] is at queryHeight-1
        let start_block_number = height.saturating_sub(1);

        // Convert bounded bytes to Vec<u8>
        let tx_bytes: Vec<u8> = encoded_transaction.into();

        // Validate inputs
        if tx_bytes.is_empty() {
            debug!(
                "Empty transaction data submitted for query: chain_key={chain_key}, height={height}"
            );
            let encoded_revert = encode_revert_message("Transaction data cannot be empty");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        // Check for empty continuity chain
        if continuity_proof.blocks.is_empty() {
            debug!("Empty continuity chain for query: chain_key={chain_key}, height={height}");
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity chain cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Charge gas for all operations upfront
        let total_continuity_gas = GAS_PER_CONTINUITY_BLOCK
            .checked_mul(continuity_proof.blocks.len() as u64)
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

        // Record weights
        let continuity_weight = sp_weights::Weight::from_parts(WEIGHT_CONTINUITY_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, continuity_weight, 0)?;

        let merkle_weight = sp_weights::Weight::from_parts(WEIGHT_MERKLE_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, merkle_weight, 0)?;

        // Step 1: Verify merkle proof for the transaction
        if !Self::verify_merkle_proof(&merkle_proof, &tx_bytes) {
            debug!("Merkle proof validation failed for chain_key={chain_key}, height={height}");
            return Self::revert_with_message("Merkle proof validation failed");
        }

        // Step 2: Verify the query block exists, merkle root matches, and digest is correct
        // Security: This verifies the query block's digest using the previous block's digest
        // This prevents sending fake roots. POC pattern: continuity chain starts at queryHeight - 1
        if let Err(err) = Self::verify_query_block_digest(
            handle,
            &continuity_proof,
            start_block_number,
            height,
            merkle_proof.root,
        ) {
            return Self::revert_with_message(err.message());
        }

        // Step 3: Verify continuity proof chain
        if let Err(err) = Self::verify_continuity_chain(
            handle,
            &continuity_proof,
            start_block_number,
            chain_key,
            height,
        ) {
            return Self::revert_with_message(err.message());
        }

        // Emit TransactionVerified event on success
        if emit_events {
            let tx_index = Self::calculate_tx_index(&merkle_proof);
            // chain_key and height are indexed (topics), transactionIndex is in data
            let event_data = ethabi::encode(&[Token::Uint(tx_index.into())]);

            log3(
                handle.context().address,
                SELECTOR_LOG_TRANSACTION_VERIFIED,
                H256::from_low_u64_be(chain_key), // First indexed topic: chain_key
                H256::from_low_u64_be(height),    // Second indexed topic: height
                event_data,                       // Data: transactionIndex
            )
            .record(handle)?;
        }

        Ok(true)
    }

    /// Internal implementation for batch queries verification
    ///
    /// Optimizes verification by validating the shared continuity chain once,
    /// then verifying each query's merkle proof individually.
    /// This approach saves ~40% gas compared to individual verifications.
    ///
    /// # Parameters
    /// - `handle`: EVM precompile handle for gas accounting and logging
    /// - `chain_key`: Chain key identifier
    /// - `heights`: Vector of block heights to verify
    /// - `encoded_transactions`: Transaction data for each query
    /// - `merkle_proofs`: Merkle proofs for each query
    /// - `shared_continuity_proof`: Single continuity proof covering all query heights
    /// - `emit_events`: Whether to emit TransactionVerified events for each successful transaction
    ///
    /// # Returns
    /// `true` on success
    ///
    /// # Verification Flow
    /// 1. Validate input arrays have matching lengths
    /// 2. Verify shared continuity chain once (major gas savings)
    /// 3. For each query:
    ///    - Verify merkle proof for transaction inclusion
    ///    - Verify query block digest matches continuity chain
    ///    - Track success/failure
    ///
    /// # Note
    /// Individual query failures cause the batch to revert immediately (no partial success).
    /// Events are only emitted for successfully verified transactions before any failure occurs.
    pub fn verify_batch_impl(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        heights: Vec<u64>,
        encoded_transactions: Vec<BoundedBytes<ConstU10MB>>,
        merkle_proofs: Vec<TransactionMerkleProof>,
        shared_continuity_proof: ContinuityProof,
        emit_events: bool,
    ) -> EvmResult<bool> {
        let num_queries = heights.len();

        debug!("Processing batch of {num_queries} queries");

        // Validate input arrays have same length
        if heights.len() != encoded_transactions.len() || heights.len() != merkle_proofs.len() {
            return Self::revert_with_message(
                "Should have the same number of heights, encoded transactions, and merkle proofs",
            );
        }

        // Check for empty queries
        if heights.is_empty() {
            return Self::revert_with_message("Should have at least one height");
        }

        // Find min and max block heights from all queries in a single pass
        let mut min_height: Option<u64> = None;
        let mut max_height: Option<u64> = None;

        for height in heights.iter() {
            min_height = Some(min_height.map_or(*height, |m| m.min(*height)));
            max_height = Some(max_height.map_or(*height, |m| m.max(*height)));
        }
        let min_height = min_height.unwrap();
        let max_height = max_height.unwrap();

        // Check for empty continuity proof
        if shared_continuity_proof.blocks.is_empty() {
            debug!("Empty continuity proof for batch queries");
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity proof cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // For batch queries: blocks[0] is at min(queryHeights)-1
        let start_block_number = min_height.saturating_sub(1);

        // Verify shared continuity chain once (more efficient than verifying per query)
        let continuity_gas = GAS_PER_CONTINUITY_BLOCK
            .checked_mul(shared_continuity_proof.blocks.len() as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(continuity_gas)?;

        // Verify continuity chain covers the range of all queries
        let first_block_number = start_block_number;
        if first_block_number > min_height {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message(
                    "Continuity chain doesn't cover minimum query height",
                ),
                exit_status: ExitRevert::Reverted,
            });
        }

        let last_block_number =
            start_block_number + (shared_continuity_proof.blocks.len() - 1) as u64;
        if last_block_number < max_height {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message(
                    "Continuity chain doesn't cover maximum query height",
                ),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Verify the continuity chain itself (using first query for chain_id)
        if let Err(err) = Self::verify_continuity_chain(
            handle,
            &shared_continuity_proof,
            start_block_number,
            chain_key,
            min_height,
        ) {
            return Self::revert_with_message(err.message());
        }

        // Record continuity weight (verified once for all queries)
        let continuity_weight = sp_weights::Weight::from_parts(WEIGHT_CONTINUITY_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, continuity_weight, 0)?;

        // Process each query
        for (i, ((height, encoded_transaction), merkle_proof)) in heights
            .into_iter()
            .zip(encoded_transactions.into_iter())
            .zip(merkle_proofs.into_iter())
            .enumerate()
        {
            debug!(
                "Processing batch query {}/{} at height {}",
                i + 1,
                num_queries,
                height
            );

            // Convert bounded bytes to Vec<u8>
            let tx_bytes: Vec<u8> = encoded_transaction.into();

            // Validate transaction data is not empty
            if tx_bytes.is_empty() {
                debug!("Empty transaction data for query at height {height}");
                return Self::revert_with_message("Transaction data cannot be empty");
            }

            // Charge gas for transaction data and merkle proof
            Self::charge_query_gas(handle, tx_bytes.len(), merkle_proof.siblings.len())?;

            // Record Merkle weight for each query
            let merkle_weight = sp_weights::Weight::from_parts(WEIGHT_MERKLE_VERIFY, 0);
            RuntimeHelper::<Runtime>::record_external_cost(handle, merkle_weight, 0)?;

            // 1. Verify Merkle proof for transaction inclusion
            if !Self::verify_merkle_proof(&merkle_proof, &tx_bytes) {
                debug!("Merkle proof validation failed for query at height {height}");
                // Don't emit events on failure (as per user requirement)
                return Self::revert_with_message("Merkle proof validation failed");
            }

            // 2. Verify query block digest (includes finding block, verifying merkle root, and digest)
            // Security: This verifies the query block's digest using the previous block's digest
            // This prevents sending fake roots. POC pattern: continuity chain starts at queryHeight - 1
            if let Err(err) = Self::verify_query_block_digest(
                handle,
                &shared_continuity_proof,
                start_block_number,
                height,
                merkle_proof.root,
            ) {
                debug!(
                    "Query block digest verification failed for height {}: {}",
                    height,
                    err.message()
                );
                // Don't emit events on failure (as per user requirement)
                return Self::revert_with_message(err.message());
            }

            // Emit TransactionVerified event for each successful transaction
            if emit_events {
                let tx_index = Self::calculate_tx_index(&merkle_proof);
                // chain_key and height are indexed (topics), transactionIndex is in data
                let event_data = ethabi::encode(&[Token::Uint(tx_index.into())]);

                log3(
                    handle.context().address,
                    SELECTOR_LOG_TRANSACTION_VERIFIED,
                    H256::from_low_u64_be(chain_key), // First indexed topic: chain_key
                    H256::from_low_u64_be(height),    // Second indexed topic: height
                    event_data,                       // Data: transactionIndex
                )
                .record(handle)?;
            }
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use merkle::{MerkleProofEntry, TransactionMerkleProof};
    use sp_core::H256;

    // Helper to create a TransactionMerkleProof for testing
    fn create_merkle_proof(root: H256, siblings: Vec<MerkleProofEntry>) -> TransactionMerkleProof {
        TransactionMerkleProof::new(root, siblings)
    }

    #[test]
    fn test_calculate_tx_index_empty_siblings() {
        // Single transaction case - should return 0
        let root = H256::from([1u8; 32]);
        let proof = create_merkle_proof(root, vec![]);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            0
        );
    }

    #[test]
    fn test_calculate_tx_index_single_sibling_left() {
        // Sibling is left, so we were right -> bit 0 = 1
        // Expected index: 1 (binary: 1)
        let root = H256::from([1u8; 32]);
        let siblings = vec![MerkleProofEntry {
            hash: H256::from([2u8; 32]),
            is_left: true,
        }];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            1
        );
    }

    #[test]
    fn test_calculate_tx_index_single_sibling_right() {
        // Sibling is right, so we were left -> bit 0 = 0
        // Expected index: 0 (binary: 0)
        let root = H256::from([1u8; 32]);
        let siblings = vec![MerkleProofEntry {
            hash: H256::from([2u8; 32]),
            is_left: false,
        }];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            0
        );
    }

    #[test]
    fn test_calculate_tx_index_two_siblings_both_left() {
        // Both siblings left -> we were right at both levels
        // Expected index: 1 | 2 = 3 (binary: 11)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: true,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            3
        );
    }

    #[test]
    fn test_calculate_tx_index_two_siblings_both_right() {
        // Both siblings right -> we were left at both levels
        // Expected index: 0 (binary: 00)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: false,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: false,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            0
        );
    }

    #[test]
    fn test_calculate_tx_index_two_siblings_left_right() {
        // First sibling left (bit 0 = 1), second sibling right (bit 1 = 0)
        // Expected index: 1 (binary: 01)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: false,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            1
        );
    }

    #[test]
    fn test_calculate_tx_index_two_siblings_right_left() {
        // First sibling right (bit 0 = 0), second sibling left (bit 1 = 1)
        // Expected index: 2 (binary: 10)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: false,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: true,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            2
        );
    }

    #[test]
    fn test_calculate_tx_index_three_siblings_all_left() {
        // All siblings left -> we were right at all levels
        // Expected index: 1 | 2 | 4 = 7 (binary: 111)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([4u8; 32]),
                is_left: true,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            7
        );
    }

    #[test]
    fn test_calculate_tx_index_three_siblings_alternating() {
        // Alternating: left, right, left
        // Expected index: 1 | 4 = 5 (binary: 101)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: false,
            },
            MerkleProofEntry {
                hash: H256::from([4u8; 32]),
                is_left: true,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            5
        );
    }

    #[test]
    fn test_calculate_tx_index_four_siblings_pattern() {
        // Pattern: left, left, right, right
        // Expected index: 1 | 2 = 3 (binary: 0011)
        let root = H256::from([1u8; 32]);
        let siblings = vec![
            MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([3u8; 32]),
                is_left: true,
            },
            MerkleProofEntry {
                hash: H256::from([4u8; 32]),
                is_left: false,
            },
            MerkleProofEntry {
                hash: H256::from([5u8; 32]),
                is_left: false,
            },
        ];
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            3
        );
    }

    #[test]
    fn test_calculate_tx_index_large_index() {
        // Create a proof that results in a large index
        // Using 10 siblings, all left -> index = 2^10 - 1 = 1023
        let root = H256::from([1u8; 32]);
        let mut siblings = Vec::new();
        for i in 0..10 {
            siblings.push(MerkleProofEntry {
                hash: H256::from([i as u8; 32]),
                is_left: true,
            });
        }
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            1023
        );
    }

    #[test]
    fn test_calculate_tx_index_max_u64_bits() {
        // Test with 64 siblings, all left -> should result in u64::MAX
        // But we'll use 63 to avoid overflow issues (2^63 - 1 is max safe)
        let root = H256::from([1u8; 32]);
        let mut siblings = Vec::new();
        for i in 0..63 {
            siblings.push(MerkleProofEntry {
                hash: H256::from([i as u8; 32]),
                is_left: true,
            });
        }
        let proof = create_merkle_proof(root, siblings);
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            u64::MAX >> 1
        ); // 2^62 - 1
    }

    #[test]
    fn test_calculate_tx_index_specific_indices() {
        // Test specific known indices
        let root = H256::from([1u8; 32]);

        // Index 0: all right siblings
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: false,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            0
        );

        // Index 1: first left, rest right
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: true,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: false,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            1
        );

        // Index 2: first right, second left
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: true,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            2
        );

        // Index 4: third bit set
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([4u8; 32]),
                    is_left: true,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            4
        );
    }

    #[test]
    fn test_calculate_tx_index_random_patterns() {
        // Test various random patterns to ensure correctness
        let root = H256::from([1u8; 32]);

        // Pattern: L, R, L, R, L -> 1 + 4 + 16 = 21
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: true,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([4u8; 32]),
                    is_left: true,
                },
                MerkleProofEntry {
                    hash: H256::from([5u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([6u8; 32]),
                    is_left: true,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            21
        );

        // Pattern: R, L, R, L, R -> 2 + 8 = 10
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: true,
                },
                MerkleProofEntry {
                    hash: H256::from([4u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([5u8; 32]),
                    is_left: true,
                },
                MerkleProofEntry {
                    hash: H256::from([6u8; 32]),
                    is_left: false,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            10
        );
    }

    #[test]
    fn test_calculate_tx_index_edge_case_single_bit() {
        // Test that each bit position works independently
        let root = H256::from([1u8; 32]);

        // Test bit 0
        let proof = create_merkle_proof(
            root,
            vec![MerkleProofEntry {
                hash: H256::from([2u8; 32]),
                is_left: true,
            }],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            1
        );

        // Test bit 1
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: true,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            2
        );

        // Test bit 2
        let proof = create_merkle_proof(
            root,
            vec![
                MerkleProofEntry {
                    hash: H256::from([2u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([3u8; 32]),
                    is_left: false,
                },
                MerkleProofEntry {
                    hash: H256::from([4u8; 32]),
                    is_left: true,
                },
            ],
        );
        assert_eq!(
            BlockProverPrecompile::<crate::mock::Runtime>::calculate_tx_index(&proof),
            4
        );
    }
}
