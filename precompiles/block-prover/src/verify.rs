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

/// Error type for continuity verification
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
    /// Get the error message
    pub fn message(&self) -> &'static str {
        match self {
            Self::InsufficientBlocks => {
                "Continuity chain must contain at least 2 blocks (queryHeight-1 and queryHeight)"
            }
            Self::QueryBlockNotFound => "Query block not found in continuity chain",
            Self::MerkleRootMismatch => "Merkle root mismatch",
            Self::PreviousBlockNotFound => "Previous block not found",
            Self::DigestMismatch => "Digest mismatch",
            Self::ChainDoesNotReachQueryHeight => "Continuity chain does not reach query height",
            Self::NoMatchingAttestationOrCheckpoint => {
                "Continuity proof does not match attestation or checkpoint"
            }
            Self::ChainLinkBroken => "Continuity chain has broken links",
        }
    }
}

// Gas cost constants
// Based on realistic Solidity implementation costs with precompile efficiency gains:
// - Keccak256 in Solidity: ~30 + 6/word, in precompile: ~10x faster
// - SLOAD: 2,100 (warm) / 2,600 (cold)
//
// Gas Model:
// - Transaction data: Calldata gas is pre-charged by EVM before reaching the precompile
// - Hash operations: Keccak-256 hash cost = 30 base + 6 per word
//   - Merkle sibling: hash_inner(left, right) = 65 bytes = 3 words = 48 gas
//   - Continuity block: hash_payload(height, root, prev_digest) = 72 bytes = 3 words = 48 gas
// Used for both Merkle sibling verification and continuity block hashing
// Gas costs properly account for all computational work, no separate weight tracking needed
pub const CONTINUITY_BLOCK_HASH_COST: u64 = 48; // Keccak-256 hash cost: 30 base + 6 per word (3-word inputs = 48 gas)

// Gas cost for a storage lookup (matches cold SLOAD)
pub const GAS_STORAGE_LOOKUP: u64 = 2_600;

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
    /// Charge gas for merkle proof verification
    /// Note: Transaction data (calldata) gas is pre-charged by EVM before reaching the precompile
    fn charge_query_gas(
        handle: &mut impl PrecompileHandle,
        _tx_bytes_len: usize, // Kept for API compatibility, but not used (calldata pre-charged)
        merkle_siblings_len: usize,
    ) -> EvmResult<()> {
        let merkle_gas = CONTINUITY_BLOCK_HASH_COST
            .checked_mul(merkle_siblings_len as u64)
            .ok_or(PrecompileFailure::Error {
                exit_status: ExitError::OutOfGas,
            })?;
        handle.record_cost(merkle_gas)?;

        Ok(())
    }

    /// Verify merkle proof for transaction inclusion
    ///
    /// Charges gas for merkle proof verification.
    /// Note: Transaction data (calldata) gas is pre-charged by EVM.
    ///
    /// # Returns
    /// `true` if the merkle proof is valid, `false` otherwise
    fn verify_merkle_proof(
        handle: &mut impl PrecompileHandle,
        merkle_proof: &TransactionMerkleProof,
        tx_bytes: &[u8],
    ) -> EvmResult<bool> {
        // Charge gas for merkle proof (calldata gas is pre-charged by EVM)
        Self::charge_query_gas(handle, tx_bytes.len(), merkle_proof.siblings.len())?;

        Ok(merkle_proof.verify(tx_bytes))
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
        // For single query: roots[0] is at queryHeight-1
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
        if continuity_proof.roots.is_empty() {
            debug!("Empty continuity chain for query: chain_key={chain_key}, height={height}");
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity chain cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Gas costs (CONTINUITY_BLOCK_HASH_COST) already account for all computational work
        // No separate weight tracking needed - gas is the single source of truth for resource consumption

        // Step 1: Verify continuity proof chain first (gas charged inside, computes digests on-chain)
        // This validates the chain structure and computes digests before we check the query block
        if let Err(err) = crate::BlockProverPrecompile::<Runtime>::verify_continuity_chain(
            handle,
            &continuity_proof,
            start_block_number,
            chain_key,
            height,
        ) {
            return Self::revert_with_message(err.message());
        }

        // Step 2: Verify merkle proof for the transaction (gas charged inside)
        if !Self::verify_merkle_proof(handle, &merkle_proof, &tx_bytes)? {
            debug!("Merkle proof validation failed for chain_key={chain_key}, height={height}");
            return Self::revert_with_message("Merkle proof validation failed");
        }

        // Step 3: Verify the query block exists and merkle root matches
        // verify_continuity_chain already verifies all block digests are correct,
        // so we just need to verify the merkle root matches
        let query_block_idx = match continuity_proof
            .find_query_block_index(start_block_number, height)
        {
            Some(idx) => idx,
            None => return Self::revert_with_message("Query block not found in continuity chain"),
        };

        let query_root = &continuity_proof.roots[query_block_idx];
        if *query_root != merkle_proof.root {
            return Self::revert_with_message("Merkle root mismatch");
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
        if shared_continuity_proof.roots.is_empty() {
            debug!("Empty continuity proof for batch queries");
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message("Continuity proof cannot be empty"),
                exit_status: ExitRevert::Reverted,
            });
        }

        // For batch queries: roots[0] is at min(queryHeights)-1
        let start_block_number = min_height.saturating_sub(1);

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
            start_block_number + (shared_continuity_proof.roots.len() - 1) as u64;
        if last_block_number < max_height {
            return Err(PrecompileFailure::Revert {
                output: encode_revert_message(
                    "Continuity chain doesn't cover maximum query height",
                ),
                exit_status: ExitRevert::Reverted,
            });
        }

        // Verify the continuity chain itself (gas charged inside)
        // For batch queries, we need to ensure the chain reaches at least max_height
        // and ends at an attestation/checkpoint
        if let Err(err) = Self::verify_continuity_chain(
            handle,
            &shared_continuity_proof,
            start_block_number,
            chain_key,
            max_height,
        ) {
            return Self::revert_with_message(err.message());
        }

        // Gas costs already account for all computational work - no separate weight tracking needed

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

            // Gas costs already account for all computational work - no separate weight tracking needed

            // 1. Verify Merkle proof for transaction inclusion (gas charged inside)
            if !Self::verify_merkle_proof(handle, &merkle_proof, &tx_bytes)? {
                debug!("Merkle proof validation failed for query at height {height}");
                // Don't emit events on failure (as per user requirement)
                return Self::revert_with_message("Merkle proof validation failed");
            }

            // 2. Verify query block exists and merkle root matches
            // verify_continuity_chain already verifies all block digests are correct,
            // so we just need to verify the merkle root matches
            let query_block_idx = match shared_continuity_proof
                .find_query_block_index(start_block_number, height)
            {
                Some(idx) => idx,
                None => {
                    return Self::revert_with_message("Query block not found in continuity chain")
                }
            };

            let query_root = &shared_continuity_proof.roots[query_block_idx];
            if *query_root != merkle_proof.root {
                debug!("Merkle root mismatch for query at height {height}");
                return Self::revert_with_message("Merkle root mismatch");
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
