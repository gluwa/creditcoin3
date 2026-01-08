// Tests for view functions

use crate::mock::*;
use crate::tests::{precompiles, setup_attestation};
use attestor_primitives::block::{Block, ContinuityProof};
use merkle::TransactionMerkleProof;
use precompile_utils::testing::*;
use sp_core::H256;

// Helper to create a simple merkle proof for a single transaction
fn create_simple_merkle_proof(tx_data: &[u8]) -> TransactionMerkleProof {
    TransactionMerkleProof {
        root: H256::from(sp_io::hashing::keccak_256(&{
            let mut prefixed = vec![0x00u8]; // LEAF_HASH_PREFIX
            prefixed.extend_from_slice(tx_data);
            prefixed
        })),
        siblings: vec![], // Single transaction, no siblings needed
    }
}

#[test]
fn test_verify_query_view_returns_same_result_as_non_view() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 100;

        // Create simple transaction data
        let tx_data = vec![42u8; 32];

        // Create a simple merkle proof for a single transaction
        let merkle_proof = create_simple_merkle_proof(&tx_data);

        // Create continuity blocks that reach the query height
        // Continuity chain starts at queryHeight (block 100, query at index 0)
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        // Create block 100 (minimum required: just the query block)
        // But we need to end at an attestation, so create blocks 100-102
        for i in 0..3 {
            let block_number = 100 + i; // Blocks 100, 101, 102
                                        // Use merkle root for block 100 so merkle verification passes
            let root = if block_number == 100 {
                merkle_proof.root
            } else {
                H256::random()
            };
            // Compute digest correctly using Block::hash_payload
            use attestor_primitives::block::Block as FragmentBlock;
            let digest = FragmentBlock::hash_payload(&block_number, &root, &prev_digest);

            continuity_blocks.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        // Setup attestation at block before first continuity block (queryHeight - 1)
        setup_attestation(1, 99, continuity_blocks[0].prev_digest);

        // Setup attestation at the end of continuity chain
        setup_attestation(1, 102, continuity_blocks.last().unwrap().digest);

        // Execute view function - should not emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    encoded_transaction: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks.clone()),
                },
            )
            .expect_no_logs() // Verify no events are emitted
            .execute_returns(true);

        // Execute non-view function - should emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_and_emit {
                    chain_key,
                    height,
                    encoded_transaction: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks.clone()),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_verify_query_view_empty_continuity_chain() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 1;

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_simple_merkle_proof(&tx_data);

        // Empty continuity blocks should cause a revert
        // Note: Empty check is redundant - find_query_block_index will return None for empty roots,
        // which fails with "Query block not found in continuity chain"
        let continuity_blocks = vec![];

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_and_emit {
                    chain_key,
                    height,
                    encoded_transaction: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| output == b"Query block not found in continuity chain");
    });
}

#[test]
fn test_verify_query_view_empty_tx_data() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 1;

        let tx_data = vec![]; // Empty transaction data
        let merkle_proof = TransactionMerkleProof {
            root: H256::random(),
            siblings: vec![],
        };

        let continuity_blocks = vec![Block {
            block_number: 1,
            root: merkle_proof.root,
            digest: H256::from_low_u64_be(1),
            prev_digest: H256::zero(),
        }];

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_and_emit {
                    chain_key,
                    height,
                    encoded_transaction: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| output == b"Transaction data cannot be empty");
    });
}

#[test]
fn test_verify_query_view_no_attestation() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 100;

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_simple_merkle_proof(&tx_data);

        // Query block at index 0 - need at least 1 block (the query block)
        // But we still need to end at an attestation, so create blocks 100-101
        let prev_digest = H256::zero(); // Would be digest of block 99

        let query_height = 100;
        let query_root = merkle_proof.root;
        let query_digest = attestor_primitives::block::Block::hash_payload(
            &query_height,
            &query_root,
            &prev_digest,
        );

        let next_height = 101;
        let next_root = H256::random();
        let next_digest = attestor_primitives::block::Block::hash_payload(
            &next_height,
            &next_root,
            &query_digest,
        );

        let continuity_blocks = vec![
            Block {
                block_number: query_height,
                root: query_root,
                prev_digest,
                digest: query_digest,
            },
            Block {
                block_number: next_height,
                root: next_root,
                prev_digest: query_digest,
                digest: next_digest,
            },
        ];

        // No attestation setup - should fail
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_and_emit {
                    chain_key,
                    height,
                    encoded_transaction: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| {
                output == b"Continuity proof does not match attestation or checkpoint"
            });
    });
}

#[test]
fn test_verify_batch_queries_view_success() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let heights: std::vec::Vec<u64> = std::vec::Vec::from([99, 100]);

        let tx_data1 = vec![1u8; 32];
        let tx_data2 = vec![2u8; 32];

        // Create simple merkle proofs
        let merkle_proof1 = create_simple_merkle_proof(&tx_data1);
        let merkle_proof2 = create_simple_merkle_proof(&tx_data2);

        // Create continuity blocks for both queries (99-102, starts at min(queryHeights))
        // Compute digests correctly using Block::hash_payload
        use attestor_primitives::block::Block as FragmentBlock;
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        // Start at min(queryHeights) = 99 (query at index 0)
        for i in 99..=102 {
            let block_number = i;
            let root = if block_number == 99 {
                merkle_proof1.root
            } else if block_number == 100 {
                merkle_proof2.root
            } else {
                H256::random()
            };
            let digest = FragmentBlock::hash_payload(&block_number, &root, &prev_digest);

            continuity_blocks.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        setup_attestation(1, 98, continuity_blocks[0].prev_digest);
        // Setup end attestation
        setup_attestation(1, 102, continuity_blocks.last().unwrap().digest);

        // Test batch view function - should not emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch {
                    chain_key,
                    heights: heights.clone().into(),
                    encoded_transactions: vec![tx_data1.clone().into(), tx_data2.clone().into()]
                        .into(),
                    merkle_proofs: vec![merkle_proof1.clone(), merkle_proof2.clone()].into(),
                    shared_continuity_proof: ContinuityProof::from_blocks(
                        continuity_blocks.clone(),
                    ),
                },
            )
            .expect_no_logs() // Verify no events are emitted for view function
            .execute_returns(true);

        // Test batch non-view function - should emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_and_emit {
                    chain_key,
                    heights: {
                        let v: std::vec::Vec<u64> = heights;
                        v.into()
                    },
                    encoded_transactions: vec![tx_data1.into(), tx_data2.into()].into(),
                    merkle_proofs: vec![merkle_proof1, merkle_proof2].into(),
                    shared_continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_verify_batch_queries_view_mixed_results() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let heights: std::vec::Vec<u64> = std::vec::Vec::from([99, 100]);

        let tx_data1 = vec![1u8; 32];
        let tx_data2 = vec![2u8; 32];

        let merkle_proof1 = create_simple_merkle_proof(&tx_data1);
        // Create an invalid merkle proof for query2
        let merkle_proof2 = TransactionMerkleProof {
            root: H256::from_low_u64_be(999), // Wrong root
            siblings: vec![],
        };

        // Create continuity blocks (99-102, starts at min(queryHeights))
        // Compute digests correctly using Block::hash_payload
        use attestor_primitives::block::Block as FragmentBlock;
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        // Start at min(queryHeights) = 99 (query at index 0)
        for i in 99..=102 {
            let block_number = i;
            let root = if block_number == 99 {
                merkle_proof1.root
            } else {
                H256::random()
            };
            let digest = FragmentBlock::hash_payload(&block_number, &root, &prev_digest);

            continuity_blocks.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        setup_attestation(1, 98, continuity_blocks[0].prev_digest);
        // Setup end attestation
        setup_attestation(1, 102, continuity_blocks.last().unwrap().digest);

        // Test batch view function - will revert on first failure (no partial success)
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_and_emit {
                    chain_key,
                    heights: {
                        let v: std::vec::Vec<u64> = heights;
                        v.into()
                    },
                    encoded_transactions: vec![tx_data1.into(), tx_data2.into()].into(),
                    merkle_proofs: vec![merkle_proof1, merkle_proof2].into(),
                    shared_continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| output == b"Merkle proof validation failed");
    });
}

#[test]
fn test_verify_batch_queries_view_mismatched_arrays() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let heights = vec![1, 2];

        let tx_data1 = vec![1u8; 32];
        // Missing second tx_data

        let merkle_proof1 = create_simple_merkle_proof(&tx_data1);
        let merkle_proof2 = create_simple_merkle_proof(&tx_data1);

        let continuity_blocks = vec![];

        // Should revert due to mismatched array lengths
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_and_emit {
                    chain_key,
                    heights: {
                        let v: std::vec::Vec<u64> = heights;
                        v.into()
                    },
                    encoded_transactions: {
                        let v: std::vec::Vec<_> = std::vec::Vec::from([tx_data1.into()]);
                        v.into()
                    }, // Only one tx_data for two queries
                    merkle_proofs: {
                        let v: std::vec::Vec<_> = std::vec::Vec::from([merkle_proof1, merkle_proof2]);
                        v.into()
                    },
                    shared_continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| output == b"Should have the same number of heights, encoded transactions, and merkle proofs");
    });
}

#[test]
fn test_verify_batch_queries_view_continuity_doesnt_cover_range() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let heights: std::vec::Vec<u64> = std::vec::Vec::from([1, 10]); // Block 10 not covered by continuity

        let tx_data1 = vec![1u8; 32];
        let tx_data2 = vec![2u8; 32];

        let merkle_proof1 = create_simple_merkle_proof(&tx_data1);
        let merkle_proof2 = create_simple_merkle_proof(&tx_data2);

        // Continuity blocks only cover blocks 1-2
        let continuity_blocks = vec![
            Block {
                block_number: 1,
                root: merkle_proof1.root,
                digest: H256::from_low_u64_be(1),
                prev_digest: H256::zero(),
            },
            Block {
                block_number: 2,
                root: H256::random(),
                digest: H256::from_low_u64_be(2),
                prev_digest: H256::from_low_u64_be(1),
            },
        ];

        setup_attestation(1, 0, H256::zero());
        // Setup end attestation at block 2
        setup_attestation(1, 2, H256::from_low_u64_be(2));

        // Should revert because continuity doesn't cover block 10
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_and_emit {
                    chain_key,
                    heights: {
                        let v: std::vec::Vec<u64> = heights;
                        v.into()
                    },
                    encoded_transactions: vec![tx_data1.into(), tx_data2.into()].into(),
                    merkle_proofs: vec![merkle_proof1, merkle_proof2].into(),
                    shared_continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| {
                output == b"Continuity chain doesn't cover maximum query height"
            });
    });
}
