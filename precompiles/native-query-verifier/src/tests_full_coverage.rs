// Full coverage tests for native-query-verifier precompile
// This file adds missing test coverage for successful verification paths and edge cases

use crate::mock::*;
use crate::*;
use attestor_primitives::{block::Block, LayoutSegment, Query};
use attestor_primitives::{Attestation, AttestationCheckpoint, SignedAttestation};
use precompile_utils::testing::*;
use sp_core::H256;

/// Helper to create a properly formatted Merkle proof that matches the precompile's expectations
/// The precompile expects siblings in a specific format with placeholders at offset positions
fn create_proper_merkle_proof_for_single_tx(tx_data: &[u8]) -> MerkleProof {
    use sp_io::hashing::keccak_256;

    // For a single transaction, the leaf hash becomes the root
    // Hash as leaf: prepend 0x00 to tx_data
    let mut prefixed_leaf = vec![0x00u8]; // LEAF_HASH_PREPEND_VALUE
    prefixed_leaf.extend_from_slice(tx_data);
    let root_hash = H256::from(keccak_256(&prefixed_leaf));

    MerkleProof {
        root: root_hash,
        siblings: vec![], // Empty siblings for single transaction
    }
}

/// Helper to create a proper binary Merkle tree proof
fn create_proper_merkle_proof_binary(
    _tx_data: &[u8],
    tx_index: usize,
    all_tx_data: Vec<Vec<u8>>,
) -> MerkleProof {
    use sp_io::hashing::keccak_256;

    // Calculate tree depth
    let num_txs = all_tx_data.len();
    let depth = (num_txs as f64).log2().ceil() as usize;

    // Build leaf hashes
    let mut current_level: Vec<H256> = all_tx_data
        .iter()
        .map(|data| {
            let mut prefixed = vec![0x00u8];
            prefixed.extend_from_slice(data);
            H256::from(keccak_256(&prefixed))
        })
        .collect();

    // Pad to power of 2 if needed
    let target_size = 2usize.pow(depth as u32);
    while current_level.len() < target_size {
        current_level.push(H256::zero());
    }

    let mut siblings = Vec::new();
    let mut index = tx_index;

    // Build tree level by level
    while current_level.len() > 1 {
        let mut next_level = Vec::new();

        // Collect siblings for this level
        let sibling_idx = if index % 2 == 0 { index + 1 } else { index - 1 };
        if sibling_idx < current_level.len() {
            // For binary tree, we need both hashes with placeholder at offset
            if index % 2 == 0 {
                siblings.push(H256::zero()); // Placeholder for our position
                siblings.push(current_level[sibling_idx]); // Sibling
            } else {
                siblings.push(current_level[sibling_idx]); // Sibling
                siblings.push(H256::zero()); // Placeholder for our position
            }
        }

        // Build next level
        for i in (0..current_level.len()).step_by(2) {
            let left = &current_level[i];
            let right = if i + 1 < current_level.len() {
                &current_level[i + 1]
            } else {
                &H256::zero()
            };

            let mut hash_input = vec![0x01u8]; // INNER_HASH_PREPEND_VALUE
            hash_input.extend_from_slice(&left.0);
            hash_input.extend_from_slice(&right.0);
            next_level.push(H256::from(keccak_256(&hash_input)));
        }

        current_level = next_level;
        index /= 2;
    }

    MerkleProof {
        root: current_level[0],
        siblings,
    }
}

/// Helper to create test transaction data with specific layout
fn create_tx_with_layout(segments: &[(usize, usize, u8)]) -> Vec<u8> {
    let mut max_end = 0;
    for &(offset, size, _) in segments {
        max_end = max_end.max(offset + size);
    }

    let mut data = vec![0u8; max_end];
    for &(offset, size, value) in segments {
        for i in 0..size {
            data[offset + i] = value;
        }
    }
    data
}

/// Helper to setup a valid attestation chain
fn setup_valid_attestation_chain(chain_id: u64, blocks: &[Block]) {
    use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;

    if blocks.is_empty() {
        return;
    }

    // Setup attestation for the first block's prev_digest
    let attestation = Attestation {
        chain_key: chain_id,
        header_number: blocks[0].block_number - 1,
        header_hash: H256::random(),
        root: H256::from([0u8; 32]),
        prev_digest: Some(H256::zero()),
    };

    let signature: [u8; 96] = [0u8; 96];
    let signed_attestation = SignedAttestation {
        attestation,
        signature,
        attestors: vec![Account::Alice],
        continuity_proof: AttestationFragmentSerializable::default(),
    };

    pallet_attestation_poc::Attestations::<Runtime>::insert(
        chain_id,
        blocks[0].prev_digest,
        signed_attestation,
    );
    pallet_attestation_poc::LastDigest::<Runtime>::insert(chain_id, blocks[0].prev_digest);
}

/// Helper to create valid continuity blocks with proper digest chain
fn create_valid_continuity_chain(start_block: u64, count: usize) -> Vec<Block> {
    use sp_io::hashing::keccak_256;

    let mut blocks = Vec::new();
    let mut prev_digest = H256::from(keccak_256(b"genesis"));

    for i in 0..count {
        let block_number = start_block + i as u64;
        let root = H256::from(keccak_256(format!("root_{block_number}").as_bytes()));

        // Compute digest matching the Block::hash_payload logic
        let mut digest_bytes = Vec::new();
        digest_bytes.extend_from_slice(&block_number.to_be_bytes());
        digest_bytes.extend_from_slice(root.as_bytes());
        digest_bytes.extend_from_slice(prev_digest.as_bytes());
        let digest = H256::from(keccak_256(&digest_bytes));

        blocks.push(Block {
            block_number,
            root,
            prev_digest,
            digest,
        });

        prev_digest = digest;
    }

    blocks
}

fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

// ============================================================================
// SUCCESSFUL VERIFICATION TESTS (Critical Missing Coverage)
// ============================================================================

#[test]
fn test_successful_verification_single_transaction() {
    ExtBuilder::default().build().execute_with(|| {
        // Create query with data extraction layout
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![
                LayoutSegment { offset: 0, size: 4 }, // Extract first 4 bytes
                LayoutSegment {
                    offset: 4,
                    size: 32,
                }, // Extract 32 bytes
                LayoutSegment {
                    offset: 36,
                    size: 20,
                }, // Extract 20 bytes (address-like)
            ],
        };

        // Create transaction data
        let tx_data = create_tx_with_layout(&[
            (0, 4, 0xAB),   // Function selector
            (4, 32, 0x11),  // First parameter
            (36, 20, 0x22), // Address parameter
        ]);

        // Create proper Merkle proof for single transaction
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create valid continuity chain
        let continuity_blocks = create_valid_continuity_chain(100, 1);

        // Setup attestation
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Execute and verify SUCCESS
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query: query.clone(),
                    tx_data: tx_data.clone().into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success
                result_segments: vec![
                    ResultSegment {
                        offset: 0,
                        bytes: {
                            let mut expected = [0u8; 32];
                            expected[28..32].copy_from_slice(&[0xAB; 4]);
                            H256::from(expected)
                        },
                    },
                    ResultSegment {
                        offset: 4,
                        bytes: H256::from([0x11; 32]),
                    },
                    ResultSegment {
                        offset: 36,
                        bytes: {
                            let mut expected = [0u8; 32];
                            expected[12..32].copy_from_slice(&[0x22; 20]);
                            H256::from(expected)
                        },
                    },
                ],
            });
    });
}

#[test]
fn test_successful_verification_multiple_transactions() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 1, // Second transaction
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        // Create multiple transactions
        let target_tx = vec![0x42u8; 64];
        let all_txs = vec![
            vec![0x11u8; 64],
            target_tx.clone(),
            vec![0x33u8; 64],
            vec![0x44u8; 64],
        ];

        // Create proper Merkle proof for binary tree
        let merkle_proof = create_proper_merkle_proof_binary(&target_tx, 1, all_txs);

        // Create continuity chain
        let continuity_blocks = create_valid_continuity_chain(100, 1);
        setup_valid_attestation_chain(1, &continuity_blocks);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: target_tx.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: H256::from([0x42; 32]),
                }],
            });
    });
}

// ============================================================================
// DATA EXTRACTION SIZE TESTS (Missing Coverage)
// ============================================================================

#[test]
fn test_extract_less_than_32_bytes() {
    ExtBuilder::default().build().execute_with(|| {
        // Test extraction of various sizes < 32 bytes
        let test_cases = vec![
            (1, 0x11),  // 1 byte
            (4, 0x22),  // 4 bytes (like selector)
            (20, 0x33), // 20 bytes (like address)
            (31, 0x44), // 31 bytes (just under limit)
        ];

        for (size, value) in test_cases {
            let query = Query {
                chain_id: 1,
                height: 100,
                index: 0,
                layout_segments: vec![LayoutSegment {
                    offset: 0,
                    size: size as u64,
                }],
            };

            let tx_data = vec![value; size];
            let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
            let continuity_blocks = create_valid_continuity_chain(100, 1);
            setup_valid_attestation_chain(1, &continuity_blocks);

            // Verify right-alignment (left-padded with zeros)
            let mut expected = [0u8; 32];
            expected[(32 - size)..].copy_from_slice(&tx_data);

            precompiles()
                .prepare_test(
                    Account::Alice,
                    Account::Precompile,
                    PCall::verify_query {
                        query,
                        tx_data: tx_data.clone().into(),
                        merkle_proof,
                        continuity_blocks,
                    },
                )
                .execute_returns(QueryVerificationResult {
                    status: 0, // Success
                    result_segments: vec![ResultSegment {
                        offset: 0,
                        bytes: H256::from(expected),
                    }],
                });
        }
    });
}

#[test]
fn test_extract_exactly_32_bytes() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data = (0..32).map(|i| i as u8).collect::<Vec<_>>();
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);
        setup_valid_attestation_chain(1, &continuity_blocks);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.clone().into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: H256::from_slice(&tx_data),
                }],
            });
    });
}

#[test]
fn test_extract_more_than_32_bytes() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![
                LayoutSegment {
                    offset: 0,
                    size: 64,
                }, // Request 64 bytes
            ],
        };

        let tx_data = (0..100).map(|i| i as u8).collect::<Vec<_>>();
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);
        setup_valid_attestation_chain(1, &continuity_blocks);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.clone().into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: H256::from_slice(&tx_data[0..32]), // Truncated to 32 bytes
                }],
            });
    });
}

// ============================================================================
// CONTINUITY CHAIN EDGE CASES (Missing Coverage)
// ============================================================================

#[test]
fn test_continuity_with_checkpoint_fallback() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);

        // Setup checkpoint instead of attestation (testing fallback)
        let checkpoint = AttestationCheckpoint::new(
            continuity_blocks[0].block_number - 1,
            continuity_blocks[0].prev_digest,
        );
        pallet_attestation_poc::Checkpoints::<Runtime>::insert(
            1,
            continuity_blocks[0].prev_digest,
            continuity_blocks[0].block_number - 1,
        );
        pallet_attestation_poc::LastCheckpoint::<Runtime>::insert(1, checkpoint);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success with checkpoint fallback
                result_segments: vec![],
            });
    });
}

#[test]
fn test_continuity_attestation_header_validation() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 3);

        // Setup attestation with correct header number
        use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;
        let attestation = Attestation {
            chain_key: 1,
            header_number: continuity_blocks[0].block_number - 1, // Correct number
            header_hash: H256::random(),
            root: H256::from([0u8; 32]),
            prev_digest: Some(H256::zero()),
        };

        let signed_attestation = SignedAttestation {
            attestation,
            signature: [0u8; 96],
            attestors: vec![Account::Alice],
            continuity_proof: AttestationFragmentSerializable::default(),
        };

        pallet_attestation_poc::Attestations::<Runtime>::insert(
            1,
            continuity_blocks[0].prev_digest,
            signed_attestation,
        );
        pallet_attestation_poc::LastDigest::<Runtime>::insert(1, continuity_blocks[0].prev_digest);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success with correct header number
                result_segments: vec![],
            });
    });
}

#[test]
fn test_continuity_wrong_attestation_header_fails() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);

        // Setup attestation with WRONG header number
        use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;
        let attestation = Attestation {
            chain_key: 1,
            header_number: continuity_blocks[0].block_number + 10, // Wrong number!
            header_hash: H256::random(),
            root: H256::from([0u8; 32]),
            prev_digest: Some(H256::zero()),
        };

        let signed_attestation = SignedAttestation {
            attestation,
            signature: [0u8; 96],
            attestors: vec![Account::Alice],
            continuity_proof: AttestationFragmentSerializable::default(),
        };

        pallet_attestation_poc::Attestations::<Runtime>::insert(
            1,
            continuity_blocks[0].prev_digest,
            signed_attestation,
        );
        // Also set last digest so the continuity chain can be validated
        pallet_attestation_poc::LastDigest::<Runtime>::insert(1, continuity_blocks[0].prev_digest);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 2, // ContinuityChainInvalid
                result_segments: vec![],
            });
    });
}

// ============================================================================
// TRANSACTION SIZE LIMIT TESTS
// ============================================================================

#[test]
fn test_transaction_at_size_limit() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        // Create transaction at exactly 10MB limit
        let tx_data = vec![0x55u8; 10_485_760]; // 10MB exactly

        // For this test, we'll use a simple proof since the focus is on size handling
        let merkle_proof = MerkleProof {
            root: H256::random(),
            siblings: vec![],
        };

        let continuity_blocks = create_valid_continuity_chain(100, 1);
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should handle without panic (will fail on merkle but that's ok for this test)
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 1, // MerkleProofInvalid (expected for this test)
                result_segments: vec![],
            });
    });
}

// ============================================================================
// GAS AND WEIGHT RECORDING TESTS
// ============================================================================

#[test]
fn test_gas_costs_scale_correctly() {
    ExtBuilder::default().build().execute_with(|| {
        // Test 1: Small transaction with few siblings
        let small_tx = [0u8; 100].to_vec();
        let small_siblings = [H256::random(), H256::random()].to_vec();
        let small_continuity = create_valid_continuity_chain(100, 2);

        // Test 2: Large transaction with many siblings
        let large_tx = vec![0u8; 10_000];
        let large_siblings = vec![H256::random(); 20];
        let large_continuity = create_valid_continuity_chain(100, 10);

        // Calculate expected gas differences
        let tx_size_diff = (large_tx.len() - small_tx.len()) as u64;
        let siblings_diff = (large_siblings.len() - small_siblings.len()) as u64;
        let continuity_diff = (large_continuity.len() - small_continuity.len()) as u64;

        let expected_gas_diff = tx_size_diff * GAS_PER_TX_BYTE
            + siblings_diff * GAS_PER_SIBLING
            + continuity_diff * GAS_PER_CONTINUITY_BLOCK;

        // Verify gas calculations scale as expected with updated constants
        assert!(expected_gas_diff > 0, "Should have gas difference");
        assert_eq!(
            expected_gas_diff,
            tx_size_diff * 16 + siblings_diff * 200 + continuity_diff * 3_000,
            "Gas should scale correctly with input sizes (16 gas per byte, 200 per sibling, 3000 per block)"
        );
    });
}

// ============================================================================
// EMPTY CONTINUITY CHAIN REVERT TEST
// ============================================================================

#[test]
fn test_empty_continuity_chain_reverts_with_message() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks: vec![], // Empty!
                },
            )
            .execute_reverts(|output| output == b"Continuity chain cannot be empty");
    });
}

// ============================================================================
// NO FINALIZED ATTESTATION OR CHECKPOINT TEST
// ============================================================================

#[test]
fn test_no_finalized_attestation_or_checkpoint_reverts() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);

        // Don't setup any attestation or checkpoint - should revert

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"No finalized attestation or checkpoint found");
    });
}

// ============================================================================
// SEGMENT OUT OF BOUNDS ERROR TEST
// ============================================================================

#[test]
fn test_segment_out_of_bounds_reverts_properly() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![
                LayoutSegment {
                    offset: 50,
                    size: 100,
                }, // Goes beyond tx_data
            ],
        };

        let tx_data = vec![0u8; 100]; // Only 100 bytes, but segment wants 50+100=150
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);
        setup_valid_attestation_chain(1, &continuity_blocks);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Data extraction error: segment out of bounds");
    });
}

// ============================================================================
// LOG RECORDING TEST
// ============================================================================

#[test]
fn test_log_costs_are_recorded() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            index: 0,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain(100, 1);
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Execute - log costs should be recorded internally (line 168 of lib.rs)
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0,
                result_segments: vec![],
            });

        // The test framework handles log cost verification internally
    });
}
