// Tests for view functions

use crate::mock::*;
use crate::tests::{precompiles, setup_attestation};
use crate::{
    BatchQueryVerificationResult, QueryVerificationResult, ResultSegment,
    SELECTOR_LOG_BATCH_QUERIES_VERIFIED, SELECTOR_LOG_QUERY_VERIFIED,
};
use attestor_primitives::query::Query;
use attestor_primitives::{block::Block, LayoutSegment};
use precompile_utils::{evm::logs::log2, testing::*};
use sp_core::H256;

// Helper to create a simple merkle proof for a single transaction
fn create_simple_merkle_proof(tx_data: &[u8]) -> crate::MerkleProof {
    crate::MerkleProof {
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
        // Create a query at block 100
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        // Create simple transaction data
        let tx_data = vec![42u8; 32];

        // Create a simple merkle proof for a single transaction
        let merkle_proof = create_simple_merkle_proof(&tx_data);

        // Create continuity blocks that reach the query height
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        for i in 97..=100 {
            let block_number = i;
            // Use merkle root for block 100 so merkle verification passes
            let root = if block_number == 100 {
                merkle_proof.root
            } else {
                H256::random()
            };
            let digest = H256::from_low_u64_be(block_number);

            continuity_blocks.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        // Setup attestation at block before first continuity block (start)
        setup_attestation(1, 96, continuity_blocks[0].prev_digest);

        // Setup attestation at the end of continuity chain
        setup_attestation(1, 100, continuity_blocks.last().unwrap().digest);

        // Execute view function - should not emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query_view {
                    query: query.clone(),
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_blocks: continuity_blocks.clone(),
                },
            )
            .expect_no_logs() // Verify no events are emitted
            .execute_returns(QueryVerificationResult {
                status: 0,
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: H256::from([42u8; 32]),
                }],
            });

        // Execute non-view function - should emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query: query.clone(),
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_blocks: continuity_blocks.clone(),
                },
            )
            .expect_log(log2(
                Account::Precompile,
                SELECTOR_LOG_QUERY_VERIFIED,
                Account::Alice,
                ethabi::encode(&[
                    ethabi::Token::FixedBytes(query.id().0.to_vec()),
                    ethabi::Token::Uint(query.chain_id.into()),
                    ethabi::Token::Uint(query.height.into()),
                    ethabi::Token::Uint(0u8.into()), // Success status
                    ethabi::Token::Array(vec![ethabi::Token::Tuple(vec![
                        ethabi::Token::Uint(0u64.into()),
                        ethabi::Token::FixedBytes(H256::from([42u8; 32]).0.to_vec()),
                    ])]),
                ]),
            ))
            .execute_returns(QueryVerificationResult {
                status: 0,
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: H256::from([42u8; 32]),
                }],
            });
    });
}

#[test]
fn test_verify_query_view_empty_continuity_chain() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 1,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_simple_merkle_proof(&tx_data);

        // Empty continuity blocks should cause a revert
        let continuity_blocks = vec![];

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query_view {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Continuity chain cannot be empty");
    });
}

#[test]
fn test_verify_query_view_empty_tx_data() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 1,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data = vec![]; // Empty transaction data
        let merkle_proof = crate::MerkleProof {
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
                PCall::verify_query_view {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Transaction data cannot be empty");
    });
}

#[test]
fn test_verify_query_view_no_attestation() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_simple_merkle_proof(&tx_data);

        let continuity_blocks = vec![Block {
            block_number: 100,
            root: merkle_proof.root,
            digest: H256::from_low_u64_be(100),
            prev_digest: H256::from_low_u64_be(99),
        }];

        // No attestation setup - should fail
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query_view {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Continuity proof does not match checkpoint");
    });
}

#[test]
fn test_verify_batch_queries_view_success() {
    ExtBuilder::default().build().execute_with(|| {
        // Create two queries at sequential blocks
        let query1 = Query {
            chain_id: 1,
            height: 99,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };
        let query2 = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data1 = vec![1u8; 32];
        let tx_data2 = vec![2u8; 32];

        // Create simple merkle proofs
        let merkle_proof1 = create_simple_merkle_proof(&tx_data1);
        let merkle_proof2 = create_simple_merkle_proof(&tx_data2);

        // Create continuity blocks for both queries
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        for i in 97..=100 {
            let block_number = i;
            let root = if block_number == 99 {
                merkle_proof1.root
            } else if block_number == 100 {
                merkle_proof2.root
            } else {
                H256::random()
            };
            let digest = H256::from_low_u64_be(block_number);

            continuity_blocks.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        setup_attestation(1, 96, continuity_blocks[0].prev_digest);
        // Setup end attestation
        setup_attestation(1, 100, continuity_blocks.last().unwrap().digest);

        // Test batch view function - should not emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries_view {
                    queries: vec![query1.clone(), query2.clone()].try_into().unwrap(),
                    tx_data_array: vec![tx_data1.clone().into(), tx_data2.clone().into()],
                    merkle_proofs: vec![merkle_proof1.clone(), merkle_proof2.clone()],
                    shared_continuity_blocks: continuity_blocks.clone(),
                },
            )
            .expect_no_logs() // Verify no events are emitted for view function
            .execute_returns(BatchQueryVerificationResult {
                successful_queries: 2,
                failed_queries: 0,
                results: vec![
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from([1u8; 32]),
                        }],
                    },
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from([2u8; 32]),
                        }],
                    },
                ],
            });

        // Test batch non-view function - should emit events
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: vec![query1.clone(), query2.clone()].try_into().unwrap(),
                    tx_data_array: vec![tx_data1.clone().into(), tx_data2.clone().into()],
                    merkle_proofs: vec![merkle_proof1.clone(), merkle_proof2.clone()],
                    shared_continuity_blocks: continuity_blocks.clone(),
                },
            )
            .expect_log(log2(
                Account::Precompile,
                SELECTOR_LOG_QUERY_VERIFIED,
                Account::Alice,
                ethabi::encode(&[
                    ethabi::Token::FixedBytes(query1.id().0.to_vec()),
                    ethabi::Token::Uint(query1.chain_id.into()),
                    ethabi::Token::Uint(query1.height.into()),
                    ethabi::Token::Uint(0u8.into()),
                    ethabi::Token::Array(vec![ethabi::Token::Tuple(vec![
                        ethabi::Token::Uint(0u64.into()),
                        ethabi::Token::FixedBytes(H256::from([1u8; 32]).0.to_vec()),
                    ])]),
                ]),
            ))
            .expect_log(log2(
                Account::Precompile,
                SELECTOR_LOG_QUERY_VERIFIED,
                Account::Alice,
                ethabi::encode(&[
                    ethabi::Token::FixedBytes(query2.id().0.to_vec()),
                    ethabi::Token::Uint(query2.chain_id.into()),
                    ethabi::Token::Uint(query2.height.into()),
                    ethabi::Token::Uint(0u8.into()),
                    ethabi::Token::Array(vec![ethabi::Token::Tuple(vec![
                        ethabi::Token::Uint(0u64.into()),
                        ethabi::Token::FixedBytes(H256::from([2u8; 32]).0.to_vec()),
                    ])]),
                ]),
            ))
            .expect_log(log2(
                Account::Precompile,
                SELECTOR_LOG_BATCH_QUERIES_VERIFIED,
                Account::Alice,
                ethabi::encode(&[
                    ethabi::Token::Uint(2u32.into()),
                    ethabi::Token::Uint(0u32.into()),
                    ethabi::Token::Uint(2u32.into()),
                ]),
            ))
            .execute_returns(BatchQueryVerificationResult {
                successful_queries: 2,
                failed_queries: 0,
                results: vec![
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from([1u8; 32]),
                        }],
                    },
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from([2u8; 32]),
                        }],
                    },
                ],
            });
    });
}

#[test]
fn test_verify_batch_queries_view_mixed_results() {
    ExtBuilder::default().build().execute_with(|| {
        // Create two queries - one will succeed, one will fail
        let query1 = Query {
            chain_id: 1,
            height: 99,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };
        let query2 = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data1 = vec![1u8; 32];
        let tx_data2 = vec![2u8; 32];

        let merkle_proof1 = create_simple_merkle_proof(&tx_data1);
        // Create an invalid merkle proof for query2
        let merkle_proof2 = crate::MerkleProof {
            root: H256::from_low_u64_be(999), // Wrong root
            siblings: vec![],
        };

        // Create continuity blocks
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        for i in 97..=100 {
            let block_number = i;
            let root = if block_number == 99 {
                merkle_proof1.root
            } else {
                H256::random()
            };
            let digest = H256::from_low_u64_be(block_number);

            continuity_blocks.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        setup_attestation(1, 96, continuity_blocks[0].prev_digest);
        // Setup end attestation
        setup_attestation(1, 100, continuity_blocks.last().unwrap().digest);

        // Test batch view function with mixed results
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries_view {
                    queries: vec![query1, query2].try_into().unwrap(),
                    tx_data_array: vec![tx_data1.into(), tx_data2.into()],
                    merkle_proofs: vec![merkle_proof1, merkle_proof2],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_returns(BatchQueryVerificationResult {
                successful_queries: 1,
                failed_queries: 1,
                results: vec![
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from([1u8; 32]),
                        }],
                    },
                    QueryVerificationResult {
                        status: 1, // MerkleProofInvalid
                        result_segments: vec![],
                    },
                ],
            });
    });
}

#[test]
fn test_verify_batch_queries_view_mismatched_arrays() {
    ExtBuilder::default().build().execute_with(|| {
        let query1 = Query {
            chain_id: 1,
            height: 1,
            layout_segments: vec![],
        };
        let query2 = Query {
            chain_id: 1,
            height: 2,
            layout_segments: vec![],
        };

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
                PCall::verify_batch_queries_view {
                    queries: vec![query1, query2].try_into().unwrap(),
                    tx_data_array: vec![tx_data1.into()], // Only one tx_data for two queries
                    merkle_proofs: vec![merkle_proof1, merkle_proof2],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Input arrays must have the same length");
    });
}

#[test]
fn test_verify_batch_queries_view_continuity_doesnt_cover_range() {
    ExtBuilder::default().build().execute_with(|| {
        let query1 = Query {
            chain_id: 1,
            height: 1,
            layout_segments: vec![],
        };
        let query2 = Query {
            chain_id: 1,
            height: 10, // Block 10 not covered by continuity
            layout_segments: vec![],
        };

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
                PCall::verify_batch_queries_view {
                    queries: vec![query1, query2].try_into().unwrap(),
                    tx_data_array: vec![tx_data1.into(), tx_data2.into()],
                    merkle_proofs: vec![merkle_proof1, merkle_proof2],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_reverts(|output| {
                output == b"Continuity chain doesn't cover maximum query height"
            });
    });
}
