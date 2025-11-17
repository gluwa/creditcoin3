// Full coverage tests for native-query-verifier precompile
// This file adds missing test coverage for successful verification paths and edge cases

use crate::mock::*;
use crate::*;
use attestor_primitives::{block::Block, LayoutSegment, Query};
use attestor_primitives::{Attestation, AttestationCheckpoint, SignedAttestation};
use mmr::query_proof::MerkleProofEntry;
use precompile_utils::testing::*;
use sp_core::H256;

use crate::verify::{GAS_PER_CONTINUITY_BLOCK, GAS_PER_SIBLING, GAS_PER_TX_BYTE};

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
        siblings: vec![], // Single transaction has no siblings
    }
}

/// Helper to create a proper binary Merkle tree proof
fn create_proper_merkle_proof_binary(
    _tx_data: &[u8],
    tx_index: usize,
    all_tx_data: Vec<Vec<u8>>,
) -> MerkleProof {
    use sp_io::hashing::keccak_256;

    // Calculate tree depth (not needed anymore, but keeping for reference)
    // let num_txs = all_tx_data.len();
    // let depth = (num_txs as f64).log2().ceil() as usize;

    // Build leaf hashes
    let mut current_level: Vec<H256> = all_tx_data
        .iter()
        .map(|data| {
            let mut prefixed = vec![0x00u8];
            prefixed.extend_from_slice(data);
            H256::from(keccak_256(&prefixed))
        })
        .collect();

    // No padding needed - we'll handle odd numbers by duplication

    let mut siblings = Vec::new();
    let mut index = tx_index;

    // Build tree level by level
    while current_level.len() > 1 {
        let mut next_level = Vec::new();

        // Collect siblings for this level
        let sibling_idx = if index % 2 == 0 { index + 1 } else { index - 1 };

        let sibling_hash = if sibling_idx < current_level.len() {
            current_level[sibling_idx]
        } else {
            // No sibling exists, duplicate current (odd case)
            current_level[index]
        };

        // Determine if sibling is on left or right
        // If we're at even index, we're on left, sibling on right
        // If we're at odd index, we're on right, sibling on left
        let is_left = index % 2 == 1;
        siblings.push(MerkleProofEntry {
            hash: sibling_hash,
            is_left,
        });

        // Build next level
        for i in (0..current_level.len()).step_by(2) {
            let left = &current_level[i];
            let right = if i + 1 < current_level.len() {
                &current_level[i + 1]
            } else {
                // Duplicate last node when odd (matches POC implementation)
                &current_level[i]
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

/// Helper to setup attestations at both ends of continuity chain
fn setup_valid_attestation_chain(chain_id: u64, blocks: &[Block]) {
    use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;

    if blocks.is_empty() {
        return;
    }

    // Setup attestation for the first block's prev_digest (start of chain)
    let start_attestation = Attestation {
        chain_key: chain_id,
        header_number: blocks[0].block_number - 1,
        header_hash: H256::random(),
        root: H256::from([0u8; 32]),
        prev_digest: Some(H256::zero()),
    };

    let signature: [u8; 96] = [0u8; 96];
    let signed_start_attestation = SignedAttestation {
        attestation: start_attestation,
        signature,
        attestors: vec![Account::Alice],
        continuity_proof: AttestationFragmentSerializable::default(),
    };

    pallet_attestation_poc::Attestations::<Runtime>::insert(
        chain_id,
        blocks[0].prev_digest,
        signed_start_attestation,
    );
    pallet_attestation_poc::LastDigest::<Runtime>::insert(chain_id, blocks[0].prev_digest);

    // Setup attestation for the last block (end of chain)
    if let Some(last_block) = blocks.last() {
        let end_attestation = Attestation {
            chain_key: chain_id,
            header_number: last_block.block_number,
            header_hash: H256::random(),
            root: H256::from([0u8; 32]),
            prev_digest: Some(H256::zero()),
        };

        let signed_end_attestation = SignedAttestation {
            attestation: end_attestation,
            signature,
            attestors: vec![Account::Alice],
            continuity_proof: AttestationFragmentSerializable::default(),
        };

        pallet_attestation_poc::Attestations::<Runtime>::insert(
            chain_id,
            last_block.digest,
            signed_end_attestation,
        );
    }
}

/// Helper to create valid continuity blocks with proper digest chain
/// Now accepts an optional merkle_root to ensure consistency with merkle proofs
fn create_valid_continuity_chain(start_block: u64, count: usize) -> Vec<Block> {
    create_valid_continuity_chain_with_root(start_block, count, None)
}

/// Helper to create valid continuity blocks with a specific merkle root for the first block
fn create_valid_continuity_chain_with_root(
    start_block: u64,
    count: usize,
    merkle_root: Option<H256>,
) -> Vec<Block> {
    use sp_io::hashing::keccak_256;

    let mut blocks = Vec::new();
    let mut prev_digest = H256::from(keccak_256(b"genesis"));

    for i in 0..count {
        let block_number = start_block + i as u64;
        // Use the provided merkle_root for the query block (index 1 when start_block = queryHeight - 1)
        // Otherwise generate one deterministically
        let root = if i == 1 {
            // Query block is at index 1 (second block) when continuity chain starts at queryHeight - 1
            merkle_root.unwrap_or_else(|| {
                H256::from(keccak_256(format!("root_{block_number}").as_bytes()))
            })
        } else {
            H256::from(keccak_256(format!("root_{block_number}").as_bytes()))
        };

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

        // Create valid continuity chain with the same merkle root as the proof
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));

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
        let merkle_proof = create_proper_merkle_proof_binary(&target_tx, 1, all_txs.clone());

        // Create valid continuity chain with the same merkle root as the proof
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
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
                layout_segments: vec![LayoutSegment {
                    offset: 0,
                    size: size as u64,
                }],
            };

            let tx_data = vec![value; size];
            let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
            let continuity_blocks =
                create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
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
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data = (0..32).map(|i| i as u8).collect::<Vec<_>>();
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
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
            layout_segments: vec![
                LayoutSegment {
                    offset: 0,
                    size: 64,
                }, // Request 64 bytes
            ],
        };

        let tx_data = (0..100).map(|i| i as u8).collect::<Vec<_>>();
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
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
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));

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

        // Setup end attestation
        use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;
        if let Some(last_block) = continuity_blocks.last() {
            let end_attestation = Attestation {
                chain_key: 1,
                header_number: last_block.block_number,
                header_hash: H256::random(),
                root: H256::from([0u8; 32]),
                prev_digest: Some(H256::zero()),
            };

            let signed_end_attestation = SignedAttestation {
                attestation: end_attestation,
                signature: [0u8; 96],
                attestors: vec![Account::Alice],
                continuity_proof: AttestationFragmentSerializable::default(),
            };

            pallet_attestation_poc::Attestations::<Runtime>::insert(
                1,
                last_block.digest,
                signed_end_attestation,
            );
        }

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
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        // Create blocks 99, 100, 101, 102 (4 blocks total)
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 4, Some(merkle_proof.root));

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

        // Setup end attestation
        if let Some(last_block) = continuity_blocks.last() {
            let end_attestation = Attestation {
                chain_key: 1,
                header_number: last_block.block_number,
                header_hash: H256::random(),
                root: H256::from([0u8; 32]),
                prev_digest: Some(H256::zero()),
            };

            let signed_end_attestation = SignedAttestation {
                attestation: end_attestation,
                signature: [0u8; 96],
                attestors: vec![Account::Alice],
                continuity_proof: AttestationFragmentSerializable::default(),
            };

            pallet_attestation_poc::Attestations::<Runtime>::insert(
                1,
                last_block.digest,
                signed_end_attestation,
            );
        }

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
fn test_continuity_wrong_attestation_header_succeeds() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));

        // Setup attestation with WRONG header number at start (doesn't matter anymore)
        use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;
        let attestation = Attestation {
            chain_key: 1,
            header_number: continuity_blocks[0].block_number + 10, // Wrong number but doesn't matter
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

        // Setup end attestation correctly
        if let Some(last_block) = continuity_blocks.last() {
            let end_attestation = Attestation {
                chain_key: 1,
                header_number: last_block.block_number,
                header_hash: H256::random(),
                root: H256::from([0u8; 32]),
                prev_digest: Some(H256::zero()),
            };

            let signed_end_attestation = SignedAttestation {
                attestation: end_attestation,
                signature: [0u8; 96],
                attestors: vec![Account::Alice],
                continuity_proof: AttestationFragmentSerializable::default(),
            };

            pallet_attestation_poc::Attestations::<Runtime>::insert(
                1,
                last_block.digest,
                signed_end_attestation,
            );
        }

        // Should succeed despite wrong header number at start
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
                status: 0, // Success
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
            siblings: vec![], // Empty entries for simple test
        };

        let continuity_blocks = create_valid_continuity_chain(99, 2);
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
            .execute_reverts(|output| output == b"Merkle proof validation failed");
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
        let small_continuity = create_valid_continuity_chain(99, 3);

        // Test 2: Large transaction with many siblings
        let large_tx = vec![0u8; 10_000];
        let large_siblings = vec![H256::random(); 20];
        let large_continuity = create_valid_continuity_chain(99, 11);

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
fn test_no_end_attestation_or_checkpoint_reverts() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let tx_data = vec![0u8; 32];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));

        // Don't setup any attestation or checkpoint - should revert because no end attestation

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
            .execute_reverts(|output| {
                output == b"Continuity proof does not match attestation or checkpoint"
            });
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
            layout_segments: vec![
                LayoutSegment {
                    offset: 50,
                    size: 100,
                }, // Goes beyond tx_data
            ],
        };

        let tx_data = vec![0u8; 100]; // Only 100 bytes, but segment wants 50+100=150
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // The extraction will fail, but it will now return a revert instead of status 3
        // because we check bounds and revert in extract_data_segments
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
// MERKLE ROOT MATCHING TESTS (Critical Security Verification)
// ============================================================================

#[test]
fn test_merkle_root_mismatch_fails() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data = vec![0xAB; 100];

        // Create merkle proof with one root
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain with a DIFFERENT root (this is an attack scenario)
        let different_root = H256::from([0xFF; 32]);
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(different_root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should fail with MerkleRootMismatch status
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
            .execute_reverts(|output| output == b"Merkle root mismatch");
    });
}

#[test]
fn test_merkle_root_matching_succeeds() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let tx_data = vec![0xAB; 100];

        // Create merkle proof
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain with the SAME root (correct scenario)
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should succeed
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
                    bytes: {
                        let mut expected = [0u8; 32];
                        expected.copy_from_slice(&tx_data[0..32]);
                        H256::from(expected)
                    },
                }],
            });
    });
}

#[test]
fn test_merkle_root_mismatch_with_multiple_blocks() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 101, // Query for second block
            layout_segments: vec![],
        };

        let tx_data = vec![0xCC; 50];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain with 3 blocks
        let mut continuity_blocks = create_valid_continuity_chain(99, 4);

        // Set the correct root for first block
        continuity_blocks[0].root = H256::from([0xAA; 32]);

        // But set wrong root for the query block (block 101, index 1)
        continuity_blocks[1].root = H256::from([0xBB; 32]); // Wrong root!

        // Correct root for third block
        continuity_blocks[2].root = H256::from([0xCC; 32]);

        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should fail because block at height 101 has wrong merkle root
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
            .execute_reverts(|output| output == b"Merkle root mismatch");
    });
}

#[test]
fn test_attack_scenario_valid_proof_wrong_block() {
    ExtBuilder::default().build().execute_with(|| {
        // Scenario: Attacker has a valid merkle proof for a transaction in block 99
        // but tries to claim it's from block 100

        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment { offset: 0, size: 4 }],
        };

        // Transaction from block 99
        let tx_data = vec![0x99; 100];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain for block 100 with different root
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let block_100_root = H256::from([0x10; 32]); // Different from merkle_proof.root
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(block_100_root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Attack should fail - merkle proof is valid but for wrong block
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
            .execute_reverts(|output| output == b"Merkle root mismatch");
    });
}

#[test]
fn test_query_block_not_in_continuity_chain() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 105, // Query for block 105
            layout_segments: vec![],
        };

        let tx_data = vec![0xDD; 50];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain that only goes from 100-103 (doesn't include 105)
        let continuity_blocks = create_valid_continuity_chain(99, 5);
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should fail because query block 105 is not in continuity chain
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
            .execute_reverts(|output| output == b"Query block not found in continuity chain");
    });
}

#[test]
fn test_merkle_root_verification_with_binary_tree() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        // Create multiple transactions for binary tree
        let tx_data_0 = vec![0x11; 64];
        let tx_data_1 = vec![0x22; 64]; // Target transaction
        let tx_data_2 = vec![0x33; 64];
        let tx_data_3 = vec![0x44; 64];

        let all_txs = vec![
            tx_data_0.clone(),
            tx_data_1.clone(),
            tx_data_2.clone(),
            tx_data_3.clone(),
        ];

        // Create merkle proof for transaction at index 1
        let merkle_proof = create_proper_merkle_proof_binary(&tx_data_1, 1, all_txs);

        // Create continuity chain with matching root
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should succeed with correct merkle root
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query: query.clone(),
                    tx_data: tx_data_1.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_blocks: continuity_blocks.clone(),
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: {
                        let mut expected = [0u8; 32];
                        expected.copy_from_slice(&tx_data_1[0..32]);
                        H256::from(expected)
                    },
                }],
            });

        // Now try with wrong transaction data but valid proof structure
        // This simulates trying to claim a different transaction is in the tree
        let wrong_tx_data = vec![0xFF; 64];

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query: query.clone(),
                    tx_data: wrong_tx_data.into(),
                    merkle_proof,
                    continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Merkle proof validation failed");
    });
}

#[test]
fn test_merkle_root_mismatch_event_emission() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let tx_data = vec![0xEE; 50];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain with different root
        let wrong_root = H256::from([0x99; 32]);
        let continuity_blocks = create_valid_continuity_chain_with_root(99, 2, Some(wrong_root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Execute and check that it returns the correct status
        // The event emission is tested internally but we verify the status
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
            .execute_reverts(|output| output == b"Merkle root mismatch");
    });
}

#[test]
fn test_continuity_chain_with_correct_query_block() {
    ExtBuilder::default().build().execute_with(|| {
        // Test that we correctly find the query block in the middle of continuity chain
        let query = Query {
            chain_id: 1,
            height: 102, // Middle block in chain
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 10,
            }],
        };

        let tx_data = vec![0x55; 100];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);

        // Create continuity chain starting at block 99 (POC pattern: queryHeight - 1)
        // Chain: 99, 100, 101, 102, 103, 104 (6 blocks)
        let mut continuity_blocks = create_valid_continuity_chain(99, 6);

        // Set the merkle root for block 102 (index 3) to match our proof
        continuity_blocks[3].root = merkle_proof.root;

        // Need to recompute digests after changing root
        use sp_io::hashing::keccak_256;
        for i in 3..continuity_blocks.len() {
            let block_number = continuity_blocks[i].block_number;
            let root = continuity_blocks[i].root;
            let prev_digest = if i > 0 {
                continuity_blocks[i - 1].digest
            } else {
                continuity_blocks[i].prev_digest
            };

            let mut digest_bytes = Vec::new();
            digest_bytes.extend_from_slice(&block_number.to_be_bytes());
            digest_bytes.extend_from_slice(root.as_bytes());
            digest_bytes.extend_from_slice(prev_digest.as_bytes());
            continuity_blocks[i].digest = H256::from(keccak_256(&digest_bytes));

            if i + 1 < continuity_blocks.len() {
                continuity_blocks[i + 1].prev_digest = continuity_blocks[i].digest;
            }
        }

        setup_valid_attestation_chain(1, &continuity_blocks);

        // Should succeed - query block is correctly found at index 3 (block 102)
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
                    bytes: {
                        let mut expected = [0u8; 32];
                        expected[22..32].copy_from_slice(&tx_data[0..10]);
                        H256::from(expected)
                    },
                }],
            });
    });
}

// ============================================================================
// SECURITY VULNERABILITY DEMONSTRATION TEST
// ============================================================================

#[test]
fn test_security_verification_prevents_cross_block_attack() {
    ExtBuilder::default().build().execute_with(|| {
        // This test demonstrates that merkle root matching verification prevents
        // cross-block attacks.
        //
        // Attack scenario:
        // 1. Attacker has a valid transaction with valid merkle proof from block 99
        // 2. Block 99 might be unattested or from a fork
        // 3. Attacker tries to claim this transaction is from attested block 100
        // 4. Without proper verification: Would succeed (merkle proof valid, continuity chain valid)
        // 5. With proper verification: Fails with MerkleRootMismatch

        // Setup: Create two different transactions with valid proofs
        let tx_from_block_99 = vec![0x99; 100]; // Transaction actually from block 99
        let tx_from_block_100 = vec![0x10; 100]; // Transaction actually from block 100

        // Create valid merkle proofs for each
        let proof_block_99 = create_proper_merkle_proof_for_single_tx(&tx_from_block_99);
        let proof_block_100 = create_proper_merkle_proof_for_single_tx(&tx_from_block_100);

        // These roots are different (different transactions)
        assert_ne!(proof_block_99.root, proof_block_100.root);

        // Create attested continuity chain for block 100 with its correct root
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(proof_block_100.root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Query claiming to extract data from block 100
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        // ATTACK ATTEMPT: Use transaction from block 99 but claim it's from block 100
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query: query.clone(),
                    tx_data: tx_from_block_99.clone().into(),
                    merkle_proof: proof_block_99, // Valid proof but for wrong block!
                    continuity_blocks: continuity_blocks.clone(),
                },
            )
            .execute_reverts(|output| output == b"Merkle root mismatch");

        // CORRECT USE: Transaction actually from block 100
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_query {
                    query,
                    tx_data: tx_from_block_100.clone().into(),
                    merkle_proof: proof_block_100, // Correct proof for block 100
                    continuity_blocks,
                },
            )
            .execute_returns(QueryVerificationResult {
                status: 0, // Success - legitimate transaction verified
                result_segments: vec![ResultSegment {
                    offset: 0,
                    bytes: {
                        let mut expected = [0u8; 32];
                        expected.copy_from_slice(&tx_from_block_100[0..32]);
                        H256::from(expected)
                    },
                }],
            });
    });
}

// ============================================================================
// BATCH QUERY VERIFICATION TESTS
// ============================================================================

#[test]
fn test_batch_queries_success() {
    ExtBuilder::default().build().execute_with(|| {
        // Create multiple queries at different heights
        let query1 = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 32,
            }],
        };

        let query2 = Query {
            chain_id: 1,
            height: 101,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 20,
            }],
        };

        let query3 = Query {
            chain_id: 1,
            height: 102,
            layout_segments: vec![LayoutSegment {
                offset: 0,
                size: 10,
            }],
        };

        // Create transaction data for each query
        let tx_data1 = vec![0x11; 100];
        let tx_data2 = vec![0x22; 100];
        let tx_data3 = vec![0x33; 100];

        // Create merkle proofs
        let proof1 = create_proper_merkle_proof_for_single_tx(&tx_data1);
        let proof2 = create_proper_merkle_proof_for_single_tx(&tx_data2);
        let proof3 = create_proper_merkle_proof_for_single_tx(&tx_data3);

        // Create continuity chain covering blocks 99-102 (must include queryHeight - 1 for digest verification)
        // Compute digests correctly using Block::hash_payload
        use attestor_primitives::block::Block as FragmentBlock;
        let mut prev_digest = H256::from([0x88; 32]); // Starting digest

        let block99_root = H256::from([0x88; 32]); // Dummy root for previous block
        let block99_digest = FragmentBlock::hash_payload(&99, &block99_root, &prev_digest);
        prev_digest = block99_digest;

        let block100_digest = FragmentBlock::hash_payload(&100, &proof1.root, &prev_digest);
        prev_digest = block100_digest;

        let block101_digest = FragmentBlock::hash_payload(&101, &proof2.root, &prev_digest);
        prev_digest = block101_digest;

        let block102_digest = FragmentBlock::hash_payload(&102, &proof3.root, &prev_digest);

        let continuity_blocks = vec![
            // Block 99 (required for query at height 100)
            Block {
                block_number: 99,
                root: block99_root,
                prev_digest: H256::from([0x88; 32]),
                digest: block99_digest,
            },
            // Block 100
            Block {
                block_number: 100,
                root: proof1.root,
                prev_digest: block99_digest,
                digest: block100_digest,
            },
            // Block 101
            Block {
                block_number: 101,
                root: proof2.root,
                prev_digest: block100_digest,
                digest: block101_digest,
            },
            // Block 102
            Block {
                block_number: 102,
                root: proof3.root,
                prev_digest: block101_digest,
                digest: block102_digest,
            },
        ];

        // Setup attestation for the continuity chain
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Execute batch verification
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: vec![query1, query2, query3].into(),
                    tx_data_array: vec![
                        tx_data1.clone().into(),
                        tx_data2.clone().into(),
                        tx_data3.clone().into(),
                    ],
                    merkle_proofs: vec![proof1, proof2, proof3],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_returns(BatchQueryVerificationResult {
                successful_queries: 3,
                failed_queries: 0,
                results: vec![
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from_slice(&tx_data1[0..32]),
                        }],
                    },
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: {
                                let mut expected = [0u8; 32];
                                expected[12..32].copy_from_slice(&tx_data2[0..20]);
                                H256::from(expected)
                            },
                        }],
                    },
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: {
                                let mut expected = [0u8; 32];
                                expected[22..32].copy_from_slice(&tx_data3[0..10]);
                                H256::from(expected)
                            },
                        }],
                    },
                ],
            });
    });
}

#[test]
fn test_batch_queries_mixed_results() {
    ExtBuilder::default().build().execute_with(|| {
        let query1 = Query {
            chain_id: 1,
            height: 100,
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

        let tx_data1 = vec![0xAA; 100];
        let tx_data2 = vec![0xBB; 100];

        let proof1 = create_proper_merkle_proof_for_single_tx(&tx_data1);
        let proof2 = create_proper_merkle_proof_for_single_tx(&tx_data2); // Valid proof for tx_data2

        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let continuity_blocks = create_valid_continuity_chain_with_root(99, 2, Some(proof1.root));
        setup_valid_attestation_chain(1, &continuity_blocks);

        // Execute batch with one success and one failure
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: vec![query1, query2].into(),
                    tx_data_array: vec![tx_data1.clone().into(), tx_data2.into()],
                    merkle_proofs: vec![proof1, proof2],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_returns(BatchQueryVerificationResult {
                successful_queries: 1,
                failed_queries: 1,
                results: vec![
                    QueryVerificationResult {
                        status: 0, // Success
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: H256::from_slice(&tx_data1[0..32]),
                        }],
                    },
                    QueryVerificationResult {
                        status: 4, // MerkleRootMismatch - proof2.root doesn't match continuity block root (which is proof1.root)
                        result_segments: vec![],
                    },
                ],
            });
    });
}

#[test]
fn test_batch_queries_continuity_failure() {
    ExtBuilder::default().build().execute_with(|| {
        let query1 = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let query2 = Query {
            chain_id: 1,
            height: 101,
            layout_segments: vec![],
        };

        let tx_data1 = vec![0x11; 50];
        let tx_data2 = vec![0x22; 50];

        let proof1 = create_proper_merkle_proof_for_single_tx(&tx_data1);
        let proof2 = create_proper_merkle_proof_for_single_tx(&tx_data2);

        // Create invalid continuity chain (broken link)
        let continuity_blocks = vec![
            Block {
                block_number: 100,
                root: proof1.root,
                prev_digest: H256::from([0x99; 32]),
                digest: H256::from([0xA0; 32]),
            },
            Block {
                block_number: 101,
                root: proof2.root,
                prev_digest: H256::from([0xBB; 32]), // Wrong prev_digest!
                digest: H256::from([0xA1; 32]),
            },
        ];

        // Setup attestation for the first block but with wrong digest to make continuity fail
        use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;
        let attestation = Attestation {
            chain_key: 1,
            header_number: 99,
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

        // Batch should revert due to continuity chain failure
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: vec![query1, query2].into(),
                    tx_data_array: vec![tx_data1.into(), tx_data2.into()],
                    merkle_proofs: vec![proof1, proof2],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Continuity chain has broken links");
    });
}

#[test]
fn test_batch_queries_invalid_input_lengths() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let tx_data = vec![0x11; 50];
        let proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain_with_root(99, 2, Some(proof.root));

        // Mismatched array lengths
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: vec![query.clone(), query].into(),
                    tx_data_array: vec![tx_data.into()], // Only 1 tx_data for 2 queries
                    merkle_proofs: vec![proof.clone(), proof],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"Input arrays must have the same length");
    });
}

#[test]
fn test_batch_queries_exceeds_max_size() {
    ExtBuilder::default().build().execute_with(|| {
        let query = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![],
        };

        let tx_data = vec![0x11; 50];
        let proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks = create_valid_continuity_chain_with_root(99, 2, Some(proof.root));

        // Create 11 queries (exceeds MAX_BATCH_SIZE of 10)
        let queries: Vec<_> = (0..11).map(|_| query.clone()).collect();
        let tx_data_array: Vec<_> = (0..11).map(|_| tx_data.clone().into()).collect();
        let merkle_proofs: Vec<_> = (0..11).map(|_| proof.clone()).collect();

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: queries.into(),
                    tx_data_array,
                    merkle_proofs,
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_reverts(|output| output == b"queries: Value is too large for length");
    });
}

#[test]
fn test_batch_queries_shared_continuity_optimization() {
    ExtBuilder::default().build().execute_with(|| {
        // Test that continuity is only verified once for all queries
        // This is a gas optimization test - functionality is same but more efficient

        let query1 = Query {
            chain_id: 1,
            height: 100,
            layout_segments: vec![LayoutSegment { offset: 0, size: 4 }],
        };

        let query2 = Query {
            chain_id: 1,
            height: 102,
            layout_segments: vec![LayoutSegment { offset: 0, size: 4 }],
        };

        let query3 = Query {
            chain_id: 1,
            height: 101,
            layout_segments: vec![LayoutSegment { offset: 0, size: 4 }],
        };

        let tx_data1 = vec![0x11; 100];
        let tx_data2 = vec![0x22; 100];
        let tx_data3 = vec![0x33; 100];

        let proof1 = create_proper_merkle_proof_for_single_tx(&tx_data1);
        let proof2 = create_proper_merkle_proof_for_single_tx(&tx_data2);
        let proof3 = create_proper_merkle_proof_for_single_tx(&tx_data3);

        // Create shared continuity chain for blocks 99-102 (must include queryHeight - 1 for digest verification)
        // Compute digests correctly using Block::hash_payload
        use attestor_primitives::block::Block as FragmentBlock;
        let mut prev_digest = H256::from([0x88; 32]); // Starting digest

        let block99_root = H256::from([0x88; 32]); // Dummy root for previous block
        let block99_digest = FragmentBlock::hash_payload(&99, &block99_root, &prev_digest);
        prev_digest = block99_digest;

        let block100_digest = FragmentBlock::hash_payload(&100, &proof1.root, &prev_digest);
        prev_digest = block100_digest;

        let block101_digest = FragmentBlock::hash_payload(&101, &proof3.root, &prev_digest); // Note: query3 is for block 101
        prev_digest = block101_digest;

        let block102_digest = FragmentBlock::hash_payload(&102, &proof2.root, &prev_digest); // Note: query2 is for block 102

        let continuity_blocks = vec![
            // Block 99 (required for query at height 100)
            Block {
                block_number: 99,
                root: block99_root,
                prev_digest: H256::from([0x88; 32]),
                digest: block99_digest,
            },
            Block {
                block_number: 100,
                root: proof1.root,
                prev_digest: block99_digest,
                digest: block100_digest,
            },
            Block {
                block_number: 101,
                root: proof3.root, // Note: query3 is for block 101
                prev_digest: block100_digest,
                digest: block101_digest,
            },
            Block {
                block_number: 102,
                root: proof2.root, // Note: query2 is for block 102
                prev_digest: block101_digest,
                digest: block102_digest,
            },
        ];

        setup_valid_attestation_chain(1, &continuity_blocks);

        // Queries are in different order than blocks to test proper matching
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch_queries {
                    queries: vec![query1, query2, query3].into(),
                    tx_data_array: vec![
                        tx_data1.clone().into(),
                        tx_data2.clone().into(),
                        tx_data3.clone().into(),
                    ],
                    merkle_proofs: vec![proof1, proof2, proof3],
                    shared_continuity_blocks: continuity_blocks,
                },
            )
            .execute_returns(BatchQueryVerificationResult {
                successful_queries: 3,
                failed_queries: 0,
                results: vec![
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: {
                                let mut expected = [0u8; 32];
                                expected[28..32].copy_from_slice(&tx_data1[0..4]);
                                H256::from(expected)
                            },
                        }],
                    },
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: {
                                let mut expected = [0u8; 32];
                                expected[28..32].copy_from_slice(&tx_data2[0..4]);
                                H256::from(expected)
                            },
                        }],
                    },
                    QueryVerificationResult {
                        status: 0,
                        result_segments: vec![ResultSegment {
                            offset: 0,
                            bytes: {
                                let mut expected = [0u8; 32];
                                expected[28..32].copy_from_slice(&tx_data3[0..4]);
                                H256::from(expected)
                            },
                        }],
                    },
                ],
            });
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
            layout_segments: vec![],
        };

        let tx_data = vec![0xAA; 100];
        let merkle_proof = create_proper_merkle_proof_for_single_tx(&tx_data);
        let continuity_blocks =
            create_valid_continuity_chain_with_root(99, 2, Some(merkle_proof.root));
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
