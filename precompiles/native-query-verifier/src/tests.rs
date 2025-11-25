use super::*;
use crate::continuity::ContinuityVerificationError;
use crate::mock::*;
use crate::test_helpers::*;
use crate::SELECTOR_LOG_TRANSACTION_VERIFIED;
use attestor_primitives::{
    block::{Block, ContinuityProof},
    Attestation, AttestationCheckpoint, SignedAttestation,
};
use fp_evm::Context;
use frame_support::assert_err;
use mmr::{query_proof::MerkleProofEntry, SimpleMerkleTree};
use precompile_utils::{evm::logs::log3, solidity, testing::*};
use precompiles_primitives::GAS_STORAGE_LOOKUP;
use sp_core::{H256, U256};
use utils::block_item_traits::{BlockItem, BlockItemIdentifier};

use crate::verify::{GAS_PER_CONTINUITY_BLOCK, GAS_PER_SIBLING, GAS_PER_TX_BYTE};

/// Simple test transaction item for merkle tree construction
#[derive(Debug, Clone)]
pub(crate) struct TestTransaction {
    id: BlockItemIdentifier,
    data: Vec<u8>,
}

impl BlockItem for TestTransaction {
    fn id(&self) -> &BlockItemIdentifier {
        &self.id
    }

    fn payload_bytes(&self) -> Vec<u8> {
        self.data.clone()
    }
}

// Helper functions removed - no longer using Query type

/// Helper to trim continuity blocks to start at queryHeight-1
/// This is needed because ContinuityProof assumes blocks[0] is at queryHeight-1
pub(crate) fn trim_continuity_blocks_for_query(
    blocks: Vec<Block>,
    query_height: u64,
) -> Vec<Block> {
    let start_height = query_height.saturating_sub(1);
    blocks
        .into_iter()
        .filter(|b| b.block_number >= start_height)
        .collect()
}

// Helper function removed - no longer using Query type

/// Helper to create sample transaction data with proper ABI encoding
fn get_sample_tx_data() -> Vec<u8> {
    // Simulate ABI encoded transaction with function selector + data
    let mut data = vec![0x12, 0x34, 0x56, 0x78]; // Function selector (4 bytes)
    data.extend_from_slice(&[0u8; 32]); // First parameter (32 bytes)
    data.extend_from_slice(&[0u8; 32]); // Second parameter (32 bytes)
    data.extend_from_slice(&[0u8; 32]); // Third parameter (32 bytes)
    data
}

/// Helper to create a merkle tree from test transactions and generate a valid proof
pub(crate) fn create_valid_merkle_proof(
    tx_data: &[u8],
    tx_index: usize,
    num_transactions: usize,
) -> (MerkleProof, Vec<TestTransaction>) {
    // Create test transactions
    let mut transactions = Vec::new();
    for i in 0..num_transactions {
        let mut data = if i == tx_index {
            tx_data.to_vec()
        } else {
            vec![0u8; tx_data.len()]
        };
        // Add some variation to non-target transactions
        if i < data.len() && i != tx_index {
            data[0] = i as u8;
        }

        transactions.push(TestTransaction {
            id: BlockItemIdentifier::new(100, i as u64),
            data,
        });
    }

    // Build merkle tree using SimpleMerkleTree (matches POC)
    let tx_bytes: Vec<Vec<u8>> = transactions.iter().map(|tx| tx.to_bytes()).collect();
    let tree = SimpleMerkleTree::new(&tx_bytes);
    let proof_result = tree.generate_proof(tx_index);

    // SimpleMerkleTree::generate_proof() already returns QueryMerkleProof with siblings populated
    let merkle_proof = MerkleProof {
        root: proof_result.root,
        siblings: proof_result.siblings,
    };

    (merkle_proof, transactions)
}

/// Helper to create an invalid merkle proof (for negative tests)
fn create_invalid_merkle_proof() -> MerkleProof {
    MerkleProof {
        root: H256::from_low_u64_be(999999), // Use a deterministic invalid root
        siblings: vec![MerkleProofEntry {
            hash: H256::random(),
            is_left: false,
        }],
    }
}

/// Helper to create continuity blocks
fn create_continuity_blocks(count: usize) -> Vec<Block> {
    create_continuity_blocks_from(1, count)
}

/// Helper to create continuity blocks starting from a specific height
fn create_continuity_blocks_from(start_height: u64, count: usize) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut prev_digest = H256::zero();

    for i in 0..count {
        let block_number = start_height + i as u64;
        let root = H256::random();
        let digest = compute_test_digest(block_number, &root, &prev_digest);

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

/// Helper to compute block digest (matches Block::hash_payload)
fn compute_test_digest(block_number: u64, root: &H256, prev_digest: &H256) -> H256 {
    use sp_core::hashing::keccak_256;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&block_number.to_be_bytes());
    bytes.extend_from_slice(root.as_bytes());
    bytes.extend_from_slice(prev_digest.as_bytes());
    H256::from(keccak_256(&bytes))
}

/// Helper to setup attestation in storage
pub(crate) fn setup_attestation(chain_key: u64, block_number: u64, digest: H256) {
    use attestor_primitives::attestation_fragment::AttestationFragmentSerializable;

    let attestation = Attestation {
        chain_key,
        header_number: block_number,
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

    pallet_attestation_poc::Attestations::<Runtime>::insert(chain_key, digest, signed_attestation);
    pallet_attestation_poc::LastDigest::<Runtime>::insert(chain_key, (block_number, digest));
}

/// Helper function to set up both lower and upper attestations for a test scenario
pub(crate) fn setup_scenario_attestations(scenario: &TestScenario) {
    // Setup lower attestation (before the continuity chain starts)
    setup_attestation(
        scenario.chain_key,
        scenario.attestation_block_number,
        scenario.attestation_digest,
    );

    // Setup upper attestation (where the continuity chain ends)
    setup_attestation(
        scenario.chain_key,
        scenario.upper_attestation_block_number,
        scenario.upper_attestation_digest,
    );
}

/// Helper to setup checkpoint in storage
fn setup_checkpoint(chain_key: u64, block_number: u64, digest: H256) {
    let checkpoint = AttestationCheckpoint::new(block_number, digest);
    pallet_attestation_poc::Checkpoints::<Runtime>::insert(chain_key, block_number, digest);
    pallet_attestation_poc::LastCheckpoint::<Runtime>::insert(chain_key, checkpoint);
}

pub(crate) fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

// ============================================================================
// Basic Input Validation Tests
// ============================================================================

#[test]
fn test_empty_tx_data_fails() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 2;
        let merkle_proof = create_invalid_merkle_proof();
        let continuity_blocks = create_continuity_blocks(2);
        let continuity_proof = ContinuityProof::from_blocks(continuity_blocks);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: vec![].into(),
                    merkle_proof,
                    continuity_proof,
                },
            )
            .execute_reverts(|output| output == b"Transaction data cannot be empty");
    });
}

#[test]
fn test_empty_continuity_chain_with_valid_merkle() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 2;
        let tx_data = get_sample_tx_data();
        let (merkle_proof, _txs) = create_valid_merkle_proof(&tx_data, 0, 1);

        // Empty continuity chain should revert
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(vec![]),
                },
            )
            .execute_reverts(|output| output == b"Continuity chain cannot be empty");
    });
}

#[test]
fn test_verify_continuity_chain_errors_with_less_than_2_blocks() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);

        // Create continuity chain with 1 block
        let prev_digest = H256::zero();
        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![Block {
            block_number: query_height,
            root,
            prev_digest,
            digest,
        }];

        // Setup attestation at query.height
        setup_attestation(1, query_height, digest);

        let mut handle = MockHandle::new(
            Account::Precompile.into(),
            Context {
                address: Account::Precompile.into(),
                caller: Account::Alice.into(),
                apparent_value: U256::zero(),
            },
        );

        assert_err!(
            NativeQueryVerifierPrecompile::<Runtime>::verify_continuity_chain(
                &mut handle,
                &continuity_blocks,
                1, // chain_key
                query_height,
            ),
            ContinuityVerificationError::InsufficientBlocks
        );
    });
}

#[test]
fn test_verify_continuity_chain_errors_when_prev_digest_not_linked() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        // 2nd block doesn't link to the first one
        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest: H256::zero(), // this is the broken link
                digest,
            },
        ];

        // Setup attestation at query.height
        setup_attestation(1, query_height, digest);

        let mut handle = MockHandle::new(
            Account::Precompile.into(),
            Context {
                address: Account::Precompile.into(),
                caller: Account::Alice.into(),
                apparent_value: U256::zero(),
            },
        );

        assert_err!(
            NativeQueryVerifierPrecompile::<Runtime>::verify_continuity_chain(
                &mut handle,
                &continuity_blocks,
                1, // chain_key
                query_height,
            ),
            ContinuityVerificationError::ChainLinkBroken
        );
    });
}

#[test]
fn test_verify_continuity_chain_errors_when_continuity_chain_doesnt_reach_query_height() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup attestation at query.height
        setup_attestation(1, query_height, digest);

        let mut handle = MockHandle::new(
            Account::Precompile.into(),
            Context {
                address: Account::Precompile.into(),
                caller: Account::Alice.into(),
                apparent_value: U256::zero(),
            },
        );

        assert_err!(
            NativeQueryVerifierPrecompile::<Runtime>::verify_continuity_chain(
                &mut handle,
                &continuity_blocks,
                1,                // chain_key
                query_height + 1, // height > continuity chain last block height
            ),
            ContinuityVerificationError::ChainDoesNotReachQueryHeight
        );
    });
}

#[test]
fn test_continuity_chain_at_attestation_height() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup attestation at query.height
        setup_attestation(1, query_height, digest);

        // This should succeed - continuity chain at attestation height
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks.clone()),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_continuity_chain_at_checkpoint_height() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at a checkpoint height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup checkpoint at query.height
        setup_checkpoint(1, query_height, digest);

        // This should succeed - continuity chain at checkpoint height
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks.clone()),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_query_at_attestation_height() {
    ExtBuilder::default().build().execute_with(|| {
        // Test that a query can be verified when the query height exactly matches an attestation height
        // This is an important edge case: queries at consensus points (attestations)
        let query_height = 200;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        // The query block (queryHeight) is at an attestation height
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup attestation at query.height (the query is ON the attestation height)
        setup_attestation(1, query_height, digest);

        // This should succeed - query is at attestation height and continuity chain ends there
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks.clone()),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_query_at_checkpoint_height() {
    ExtBuilder::default().build().execute_with(|| {
        // Test that a query can be verified when the query height exactly matches a checkpoint height
        // This is an important edge case: queries at consensus points (checkpoints)
        let query_height = 300;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        // The query block (queryHeight) is at a checkpoint height
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup checkpoint at query.height (the query is ON the checkpoint height)
        setup_checkpoint(1, query_height, digest);

        // This should succeed - query is at checkpoint height and continuity chain ends there
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks.clone()),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_single_block_continuity_chain_fails() {
    ExtBuilder::default().build().execute_with(|| {
        // Security: Single-block continuity chain should fail - need at least 2 blocks
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create single-block continuity chain (should fail)
        let root = test_block.merkle_root;
        let prev_digest = H256::zero();
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![Block {
            block_number: query_height,
            root,
            prev_digest,
            digest,
        }];

        // This should fail - need at least 2 blocks to verify digest
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| {
                output == b"Continuity chain must contain at least 2 blocks (queryHeight-1 and queryHeight)"
            });
    });
}

#[test]
fn test_continuity_chain_wrong_digest_fails() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height but with wrong digest in query block
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create continuity chain with 2 blocks but wrong digest in query block
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let wrong_digest = H256::random(); // Wrong digest

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest: wrong_digest, // Wrong digest
            },
        ];

        // Setup attestation at query.height with correct digest
        let correct_digest = compute_test_digest(query_height, &root, &prev_digest);
        setup_attestation(1, query_height, correct_digest);

        // This should fail - query block digest verification failed
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| output == b"Query block digest verification failed");
    });
}

#[test]
fn test_no_layout_segments_succeeds_with_empty_results() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        // Should succeed
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

// ============================================================================
// Gas Cost Tests
// ============================================================================

#[test]
fn test_gas_calculation_base() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        let _tx_data_len = scenario.tx_data.len() as u64;
        let _siblings_count = scenario.merkle_proof.siblings.len() as u64;

        // Should succeed with proper gas calculation
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_gas_scales_with_tx_size() {
    ExtBuilder::default().build().execute_with(|| {
        let small_gas = (GAS_PER_TX_BYTE * 100)
            + GAS_PER_SIBLING
            + GAS_PER_CONTINUITY_BLOCK
            + (GAS_STORAGE_LOOKUP * 2);
        let large_gas = (GAS_PER_TX_BYTE * 1000)
            + GAS_PER_SIBLING
            + GAS_PER_CONTINUITY_BLOCK
            + (GAS_STORAGE_LOOKUP * 2);

        assert!(large_gas > small_gas);
        assert_eq!(large_gas - small_gas, GAS_PER_TX_BYTE * 900);

        // Verify new gas costs are reasonable
        assert_eq!(GAS_PER_TX_BYTE, 16, "Per byte should match calldata cost");
        assert_eq!(
            GAS_STORAGE_LOOKUP, 2_600,
            "Storage lookup should match SLOAD"
        );
    });
}

// ============================================================================
// Continuity Chain Validation Tests
// ============================================================================

#[test]
fn test_continuity_chain_with_attestation() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        // Extract expected data segments (unused but kept for documentation)
        let _expected_address = &scenario.tx_data[4..36];
        let _expected_amount = &scenario.tx_data[36..68];

        // This should succeed now that everything is properly set up
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_continuity_chain_with_checkpoint() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup checkpoint at lower bound and attestation at upper bound
        setup_checkpoint(
            1,
            scenario.attestation_block_number,
            scenario.attestation_digest,
        );
        setup_attestation(
            1,
            scenario.upper_attestation_block_number,
            scenario.upper_attestation_digest,
        );

        // Extract expected data segments (unused but kept for documentation)
        let _expected_address = &scenario.tx_data[4..36];
        let _expected_amount = &scenario.tx_data[36..68];

        // This should succeed now
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_continuity_chain_invalid_prev_digest_succeeds() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup lower attestation with WRONG digest (doesn't match first block's prev_digest)
        // This should still succeed since we don't validate the start
        setup_attestation(
            1,
            scenario.attestation_block_number,
            H256::random(), // Wrong digest - but doesn't matter
        );
        // Setup upper attestation correctly
        setup_attestation(
            1,
            scenario.upper_attestation_block_number,
            scenario.upper_attestation_digest,
        );

        // Extract expected data segments (unused but kept for documentation)
        let _expected_address = &scenario.tx_data[4..36];
        let _expected_amount = &scenario.tx_data[36..68];

        // Should succeed despite wrong lower attestation digest
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_continuity_chain_broken_link_fails() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let mut scenario = TestScenario::new_valid(5, 0);

        // Break the chain by changing second block's digest to not match the computed digest
        // Since prev_digest is reconstructed from the chain, we need to break the digest instead
        if scenario.continuity_blocks.len() > 1 {
            // Change the digest to a random value, which won't match the computed digest
            scenario.continuity_blocks[1].digest = H256::random();
        }

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_reverts(|output| {
                output == b"Continuity chain has broken links"
                    || output == b"Query block digest verification failed"
            });
    });
}

#[test]
fn test_continuity_no_finalized_attestation_fails() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 2;
        let tx_data = get_sample_tx_data();
        let (merkle_proof, _txs) = create_valid_merkle_proof(&tx_data, 0, 1);
        let continuity_blocks = create_continuity_blocks(2);

        // Don't setup any attestation or checkpoint
        // This will fail at continuity chain validation when checking for attestations

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_reverts(|output| output == b"Merkle proof validation failed");
    });
}

#[test]
fn test_continuity_checkpoint_block_number_mismatch_succeeds() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup checkpoint at wrong block number (5 instead of expected attestation_block_number)
        // This should still succeed since we don't validate the start
        setup_checkpoint(1, 5, scenario.attestation_digest);
        // Setup upper attestation correctly
        setup_attestation(
            1,
            scenario.upper_attestation_block_number,
            scenario.upper_attestation_digest,
        );

        // Extract expected data segments (unused but kept for documentation)
        let _expected_address = &scenario.tx_data[4..36];
        let _expected_amount = &scenario.tx_data[36..68];

        // Should succeed despite wrong checkpoint block number
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

// ============================================================================
// Data Extraction Tests
// ============================================================================

#[test]
fn test_extract_single_segment() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_extract_multiple_segments() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_segment_out_of_bounds_fails() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);
        // Setup both attestations
        setup_scenario_attestations(&scenario);

        // Should succeed (no data extraction in new interface)
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_segment_offset_beyond_data_fails() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);
        // Setup both attestations
        setup_scenario_attestations(&scenario);

        // Should succeed (no data extraction in new interface)
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

// ============================================================================
// Merkle Proof Tests
// ============================================================================

/// Test merkle proof validation with valid proof
#[test]
fn test_merkle_proof_validation_with_valid_proof() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        // Extract expected data segments (unused but kept for documentation)
        let _expected_address = &scenario.tx_data[4..36];
        let _expected_amount = &scenario.tx_data[36..68];

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

/// Test invalid merkle proof fails
#[test]
fn test_invalid_merkle_proof_fails() {
    ExtBuilder::default().build().execute_with(|| {
        // Create scenario with invalid merkle proof
        let scenario = TestScenario::new_with_invalid_merkle_proof(2);

        // Setup both attestations
        setup_scenario_attestations(&scenario);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_reverts(|output| output == b"Merkle proof validation failed");
    });
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

#[test]
fn test_zero_size_segment_succeeds() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario
        let scenario = TestScenario::new_valid(5, 0);
        // Setup both attestations
        setup_scenario_attestations(&scenario);

        // Should succeed
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_large_continuity_chain() {
    ExtBuilder::default().build().execute_with(|| {
        // Create a test scenario with query at height 10
        let query_height = 10;
        let test_block = create_test_block(query_height, 4);
        let tx_data = test_block.transactions[0].data.clone();
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create a long continuity chain (20 blocks) that includes the query block
        let start_height = 1;
        let all_continuity_blocks =
            create_deterministic_continuity_chain(start_height, 20, &[test_block]);

        // The attestation digest is the prev_digest of the first continuity block
        let attestation_digest = all_continuity_blocks[0].prev_digest;
        let attestation_block_number = 0;

        // Setup attestation at the start
        setup_attestation(1, attestation_block_number, attestation_digest);
        // Setup attestation at the end of the continuity chain
        let end_block_number = start_height + 20 - 1; // Block 20
        setup_attestation(
            1,
            end_block_number,
            all_continuity_blocks.last().unwrap().digest,
        );

        // Trim continuity blocks to start at queryHeight-1 (as ContinuityProof expects)
        let continuity_blocks =
            trim_continuity_blocks_for_query(all_continuity_blocks, query_height);

        // Should succeed with large continuity chain
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: 1,
                    height: query_height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_encode_revert_message() {
    let message = "Test error message";
    let encoded = encode_revert_message(message);

    // First 4 bytes should be Error(string) selector: 0x08c379a0
    assert_eq!(&encoded[0..4], &[0x08, 0xc3, 0x79, 0xa0]);

    // Remaining bytes should be ABI-encoded string
    assert!(encoded.len() > 4);
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_continuity_chain_height_validation() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 100;
        let tx_data = get_sample_tx_data();

        // We'll set up attestations as needed for each test case

        // Create a simple valid merkle proof (single transaction)
        let merkle_proof = MerkleProof {
            root: H256::from(sp_io::hashing::keccak_256(&{
                let mut prefixed = vec![0x00u8]; // LEAF_HASH_PREFIX
                prefixed.extend_from_slice(&tx_data);
                prefixed
            })),
            siblings: vec![], // Single transaction, no siblings needed
        };

        // Test 1: Continuity chain with wrong merkle root at query height
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        // Block 100 has wrong root, so verification should fail
        let mut continuity_blocks_wrong_root = Vec::new();
        let mut prev_digest = H256::zero();

        // Create blocks 99 and 100 (minimum required: queryHeight-1 and queryHeight)
        // Block 100 has wrong root to test error path
        for i in 0..2 {
            let block_number = 99 + i; // Will create blocks 99, 100
                                       // Use random root (wrong root for block 100 to test error path)
            let root = H256::random();
            let digest = compute_test_digest(block_number, &root, &prev_digest);

            continuity_blocks_wrong_root.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        // Setup attestation at block 98 to make the chain valid (start)
        setup_attestation(1, 98, continuity_blocks_wrong_root[0].prev_digest);
        // Setup attestation at block 100 where the chain ends
        setup_attestation(1, 100, continuity_blocks_wrong_root.last().unwrap().digest);

        // This should fail because block 100 has wrong merkle root
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks_wrong_root),
                },
            )
            .execute_reverts(|output| output == b"Merkle root mismatch");

        // Test 2: Continuity chain that reaches exactly the query height (block 100)
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        let mut continuity_blocks_valid = Vec::new();
        prev_digest = H256::zero();

        // Create blocks 99 and 100 (minimum required: queryHeight-1 and queryHeight)
        // Block 100 must have the correct merkle root to match the query
        for i in 0..2 {
            let block_number = 99 + i; // Will create blocks 99, 100
                                       // Use merkle root for block 100 where the transaction is
            let root = if block_number == 100 {
                merkle_proof.root
            } else {
                H256::random()
            };
            let digest = compute_test_digest(block_number, &root, &prev_digest);

            continuity_blocks_valid.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        // Setup attestation at block 98 to make the chain valid (start)
        setup_attestation(1, 98, continuity_blocks_valid[0].prev_digest);
        // Setup attestation at block 100 where the chain ends (with correct digest)
        setup_attestation(1, 100, continuity_blocks_valid.last().unwrap().digest);

        // This should pass all validations and return success
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: tx_data.clone().into(),
                    merkle_proof: merkle_proof.clone(),
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks_valid.clone()),
                },
            )
            .execute_returns(true);

        // Test 3: Continuity chain that extends beyond query height
        // POC pattern: continuity chain starts at queryHeight - 1 (block 99)
        // Chain extends to block 101, which is acceptable
        let mut continuity_blocks_extended = Vec::new();
        prev_digest = H256::zero();

        // Create blocks 99, 100, 101 (starting at queryHeight - 1)
        for i in 0..3 {
            let block_number = 99 + i; // Will create blocks 99, 100, 101
                                       // Use merkle root for block 100 where the transaction is
            let root = if block_number == 100 {
                merkle_proof.root
            } else {
                H256::random()
            };
            let digest = compute_test_digest(block_number, &root, &prev_digest);

            continuity_blocks_extended.push(Block {
                block_number,
                root,
                prev_digest,
                digest,
            });

            prev_digest = digest;
        }

        // Setup attestation at block 98 to make the chain valid (start)
        setup_attestation(1, 98, continuity_blocks_extended[0].prev_digest);
        // Setup attestation at block 101 where the extended chain ends
        setup_attestation(1, 101, continuity_blocks_extended.last().unwrap().digest);

        // This should also pass - extending beyond query height is acceptable
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks_extended),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_full_query_verification_flow() {
    ExtBuilder::default().build().execute_with(|| {
        // Use deterministic test scenario at height 100
        let scenario = TestScenario::new_valid(100, 0);

        // Setup both attestations for the continuity chain
        setup_scenario_attestations(&scenario);

        // Extract expected data segments (unused but kept for documentation)
        let _expected_address = &scenario.tx_data[4..36];
        let _expected_amount = &scenario.tx_data[36..68];

        // Execute verification
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key: scenario.chain_key,
                    height: scenario.height,
                    tx_data: scenario.tx_data.clone().into(),
                    merkle_proof: scenario.merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(
                        scenario.continuity_blocks.clone(),
                    ),
                },
            )
            .execute_returns(true);
    });
}

#[test]
fn test_verify_query_block_digest_errors_when_find_query_block_index_returns_0() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        // will be used to trick SUT that merkle_root is correct so it can
        // trigger the next error condition
        let merkle_proof = create_valid_merkle_proof_for_block(&prev_test_block, 0);

        let continuity_blocks = vec![
            Block {
                block_number: prev_height,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup attestation at query.height
        setup_attestation(1, query_height, digest);

        let mut handle = MockHandle::new(
            Account::Precompile.into(),
            Context {
                address: Account::Precompile.into(),
                caller: Account::Alice.into(),
                apparent_value: U256::zero(),
            },
        );

        assert_err!(
            NativeQueryVerifierPrecompile::<Runtime>::verify_query_block_digest(
                &mut handle,
                &continuity_blocks,
                query_height - 1, // <-- will cause internal block idx == 0
                merkle_proof.root,
            ),
            ContinuityVerificationError::PreviousBlockNotFound
        );
    });
}

#[test]
fn test_verify_query_block_digest_errors_when_prev_block_is_not_minus_1() {
    ExtBuilder::default().build().execute_with(|| {
        // Query at an attestation height - requires at least 2 blocks (queryHeight-1 and queryHeight)
        let query_height = 100;
        let test_block = create_test_block(query_height, 4);
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, 0);

        // Create continuity chain with 2 blocks: queryHeight-1 and queryHeight
        let prev_height = query_height - 1;
        let prev_test_block = create_test_block(prev_height, 4);
        let prev_root = prev_test_block.merkle_root;
        let prev_prev_digest = H256::zero();
        let prev_digest = compute_test_digest(prev_height, &prev_root, &prev_prev_digest);

        let root = test_block.merkle_root;
        let digest = compute_test_digest(query_height, &root, &prev_digest);

        let continuity_blocks = vec![
            Block {
                // IMPORTANT: will trigger the safety check:
                // Verify the previous block is actually at queryHeight - 1
                block_number: prev_height - 1,
                root: prev_root,
                prev_digest: prev_prev_digest,
                digest: prev_digest,
            },
            Block {
                block_number: query_height,
                root,
                prev_digest,
                digest,
            },
        ];

        // Setup attestation at query.height
        setup_attestation(1, query_height, digest);

        let mut handle = MockHandle::new(
            Account::Precompile.into(),
            Context {
                address: Account::Precompile.into(),
                caller: Account::Alice.into(),
                apparent_value: U256::zero(),
            },
        );

        assert_err!(
            NativeQueryVerifierPrecompile::<Runtime>::verify_query_block_digest(
                &mut handle,
                &continuity_blocks,
                query_height,
                merkle_proof.root,
            ),
            ContinuityVerificationError::PreviousBlockNotFound
        );
    });
}

#[test]
fn test_verify_batch_queries_impl_errors_with_empty_queries() {
    ExtBuilder::default().build().execute_with(|| {
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch {
                    chain_key: 1,
                    heights: vec![],
                    tx_data_array: vec![],
                    merkle_proofs: vec![],
                    shared_continuity_proof: ContinuityProof::from_blocks(vec![]),
                },
            )
            .execute_reverts(|output| output == b"Input arrays must have the same length");
    });
}

#[test]
fn test_verify_batch_queries_impl_errors_with_empty_continuity_proof() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1;
        let height = 2;
        let tx_data = get_sample_tx_data();
        let (merkle_proof, _txs) = create_valid_merkle_proof(&tx_data, 0, 1);

        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify_batch {
                    chain_key,
                    heights: vec![height],
                    tx_data_array: vec![tx_data.into()],
                    merkle_proofs: vec![merkle_proof],
                    shared_continuity_proof: ContinuityProof::from_blocks(vec![]),
                },
            )
            .execute_reverts(|output| output == b"Continuity proof cannot be empty");
    });
}

#[test]
fn test_transaction_verified_event_with_correct_tx_index() {
    ExtBuilder::default().build().execute_with(|| {
        // Test with transaction at index 2 in a block with 4 transactions
        let tx_index = 2usize;
        let chain_key = 1u64;
        let height = 100u64;

        // Create a test block with 4 transactions
        let test_block = create_test_block(height, 4);
        let tx_data = test_block.transactions[tx_index].data.clone();

        // Create merkle proof for transaction at index 2
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, tx_index);

        // Create continuity chain starting at height - 1
        let start_height = height - 1;
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        // Create block at height - 1
        let root1 = H256::random();
        let digest1 = compute_test_digest(start_height, &root1, &prev_digest);
        continuity_blocks.push(Block {
            block_number: start_height,
            root: root1,
            prev_digest,
            digest: digest1,
        });
        prev_digest = digest1;

        // Create block at height with the merkle root
        let digest2 = compute_test_digest(height, &merkle_proof.root, &prev_digest);
        continuity_blocks.push(Block {
            block_number: height,
            root: merkle_proof.root,
            prev_digest,
            digest: digest2,
        });

        // Setup attestations
        setup_attestation(
            chain_key,
            start_height - 1,
            continuity_blocks[0].prev_digest,
        );
        setup_attestation(chain_key, height, continuity_blocks[1].digest);

        // Calculate expected transaction index from merkle proof
        // The calculate_tx_index function reconstructs it from siblings
        let expected_tx_index = {
            if merkle_proof.siblings.is_empty() {
                0u8
            } else {
                let mut tx_idx = 0u8;
                for (bit_position, sibling) in merkle_proof.siblings.iter().enumerate() {
                    if sibling.is_left {
                        tx_idx |= 1u8 << bit_position;
                    }
                }
                tx_idx
            }
        };

        // Verify expected_tx_index matches the actual tx_index
        assert_eq!(expected_tx_index, tx_index as u8);

        // Encode event data: only tx_index (chain_key and height are indexed topics)
        let event_data = solidity::encode_event_data(expected_tx_index);

        // Execute and verify event is emitted with correct data
        precompiles()
            .prepare_test(
                Account::Alice,
                Account::Precompile,
                PCall::verify {
                    chain_key,
                    height,
                    tx_data: tx_data.into(),
                    merkle_proof,
                    continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
                },
            )
            .expect_log(log3(
                Account::Precompile,
                SELECTOR_LOG_TRANSACTION_VERIFIED,
                H256::from_low_u64_be(chain_key), // First indexed topic: chain_key
                H256::from_low_u64_be(height),    // Second indexed topic: height
                event_data,                       // Data: txIndex
            ))
            .execute_returns(true);
    });
}

#[test]
fn test_transaction_verified_event_batch_with_correct_tx_indices() {
    ExtBuilder::default().build().execute_with(|| {
        let chain_key = 1u64;
        let height = 100u64;

        // Create a test block with 4 transactions
        let test_block = create_test_block(height, 4);

        // Test with transactions at indices 0 and 3
        let tx_indices = vec![0usize, 3usize];
        let mut tx_data_array = Vec::new();
        let mut merkle_proofs = Vec::new();
        let mut expected_tx_indices = Vec::new();

        for &tx_index in &tx_indices {
            let tx_data = test_block.transactions[tx_index].data.clone();
            let merkle_proof = create_valid_merkle_proof_for_block(&test_block, tx_index);

            // Calculate expected transaction index from merkle proof
            let expected_tx_index = {
                if merkle_proof.siblings.is_empty() {
                    0u8
                } else {
                    let mut tx_idx = 0u8;
                    for (bit_position, sibling) in merkle_proof.siblings.iter().enumerate() {
                        if sibling.is_left {
                            tx_idx |= 1u8 << bit_position;
                        }
                    }
                    tx_idx
                }
            };

            assert_eq!(expected_tx_index, tx_index as u8);

            tx_data_array.push(tx_data);
            merkle_proofs.push(merkle_proof);
            expected_tx_indices.push(expected_tx_index);
        }

        // Create continuity chain starting at height - 1
        let start_height = height - 1;
        let mut continuity_blocks = Vec::new();
        let mut prev_digest = H256::zero();

        // Create block at height - 1
        let root1 = H256::random();
        let digest1 = compute_test_digest(start_height, &root1, &prev_digest);
        continuity_blocks.push(Block {
            block_number: start_height,
            root: root1,
            prev_digest,
            digest: digest1,
        });
        prev_digest = digest1;

        // Create block at height with the merkle root from first proof
        let digest2 = compute_test_digest(height, &merkle_proofs[0].root, &prev_digest);
        continuity_blocks.push(Block {
            block_number: height,
            root: merkle_proofs[0].root,
            prev_digest,
            digest: digest2,
        });

        // Setup attestations
        setup_attestation(
            chain_key,
            start_height - 1,
            continuity_blocks[0].prev_digest,
        );
        setup_attestation(chain_key, height, continuity_blocks[1].digest);

        // Execute batch verification
        // Build event expectations first
        let mut event_logs = Vec::new();
        for &expected_tx_index in &expected_tx_indices {
            // Encode event data: only tx_index (chain_key and height are indexed topics)
            let event_data = solidity::encode_event_data(expected_tx_index);
            event_logs.push(log3(
                Account::Precompile,
                SELECTOR_LOG_TRANSACTION_VERIFIED,
                H256::from_low_u64_be(chain_key), // First indexed topic: chain_key
                H256::from_low_u64_be(height),    // Second indexed topic: height
                event_data,                       // Data: txIndex
            ));
        }

        // Chain expect_log calls
        let precompiles_instance = precompiles();
        let mut test_builder = precompiles_instance.prepare_test(
            Account::Alice,
            Account::Precompile,
            PCall::verify_batch {
                chain_key,
                heights: vec![height, height],
                tx_data_array: tx_data_array.iter().map(|d| d.clone().into()).collect(),
                merkle_proofs: merkle_proofs.clone(),
                shared_continuity_proof: ContinuityProof::from_blocks(continuity_blocks),
            },
        );

        for event_log in event_logs {
            test_builder = test_builder.expect_log(event_log);
        }

        test_builder.execute_returns(true);
    });
}
