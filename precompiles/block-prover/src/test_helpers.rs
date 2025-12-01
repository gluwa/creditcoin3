// Test helpers for creating deterministic test data for native query verifier
use crate::*;
use attestor_primitives::block::Block;
use merkle::{KeccakMerkleTree, MerkleProofEntry, TransactionMerkleProof};
use sp_core::H256;
use sp_io::hashing::keccak_256;

/// Represents a deterministic test transaction
#[derive(Clone, Debug)]
pub struct TestTransaction {
    pub data: Vec<u8>,
}

/// Represents a deterministic test block with transactions
#[derive(Clone, Debug)]
pub struct TestBlock {
    pub block_number: u64,
    pub transactions: Vec<TestTransaction>,
    pub merkle_root: H256,
}

/// Creates deterministic transaction data for testing
pub fn create_deterministic_tx_data(index: u64) -> Vec<u8> {
    let mut data = vec![0u8; 100];
    // Function selector (4 bytes)
    data[0..4].copy_from_slice(&[0x12, 0x34, 0x56, 0x78]);
    // Address field (32 bytes) - deterministic based on index
    for i in 0..32 {
        data[4 + i] = ((index + 1) * (i as u64 + 1) % 256) as u8;
    }
    // Amount field (32 bytes) - deterministic based on index
    for i in 0..32 {
        data[36 + i] = ((index + 2) * (i as u64 + 1) % 256) as u8;
    }
    // Rest of data
    for (i, byte) in data.iter_mut().enumerate().take(100).skip(68) {
        *byte = ((index + 3) * (i as u64 + 1) % 256) as u8;
    }
    data
}

/// Creates a deterministic test block with transactions
pub fn create_test_block(block_number: u64, num_transactions: usize) -> TestBlock {
    let mut transactions = Vec::new();
    let mut tx_data_vec = Vec::new();

    for i in 0..num_transactions {
        let tx_data = create_deterministic_tx_data(block_number * 1000 + i as u64);
        transactions.push(TestTransaction {
            data: tx_data.clone(),
        });
        tx_data_vec.push(tx_data);
    }

    // Build merkle tree using KeccakMerkleTree (which duplicates last node for odd counts)
    let tree = KeccakMerkleTree::new(&tx_data_vec);
    let merkle_root = tree.root();

    TestBlock {
        block_number,
        transactions,
        merkle_root,
    }
}

/// Creates a valid merkle proof for a transaction in a test block
pub fn create_valid_merkle_proof_for_block(
    block: &TestBlock,
    tx_index: usize,
) -> TransactionMerkleProof {
    let tx_data: Vec<Vec<u8>> = block
        .transactions
        .iter()
        .map(|tx| tx.data.clone())
        .collect();

    let tree = KeccakMerkleTree::new(&tx_data);
    let proof = tree
        .generate_proof(tx_index)
        .expect("Failed to generate proof");

    // Convert to TransactionMerkleProof format
    let siblings = proof
        .siblings
        .into_iter()
        .map(|sibling| MerkleProofEntry {
            hash: sibling.hash,
            is_left: sibling.is_left,
        })
        .collect();

    TransactionMerkleProof {
        root: block.merkle_root,
        siblings,
    }
}

/// Creates a deterministic continuity chain with proper digests
pub fn create_deterministic_continuity_chain(
    start_height: u64,
    count: usize,
    blocks: &[TestBlock],
) -> Vec<Block> {
    let mut continuity_blocks = Vec::new();
    let mut prev_digest = H256::from_low_u64_be(0); // Start with a known digest

    for i in 0..count {
        let block_number = start_height + i as u64;

        // Find the corresponding test block if it exists
        let root = blocks
            .iter()
            .find(|b| b.block_number == block_number)
            .map(|b| b.merkle_root)
            .unwrap_or_else(|| {
                // Create a deterministic root for blocks without test data
                H256::from_low_u64_be(block_number * 12345)
            });

        // Compute digest using Block::hash_payload to match production code
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

    continuity_blocks
}

/// Creates a complete test scenario with block, transaction, merkle proof, and continuity chain
pub struct TestScenario {
    pub chain_key: u64,
    pub height: u64,
    pub tx_data: Vec<u8>,
    pub merkle_proof: TransactionMerkleProof,
    pub continuity_blocks: Vec<Block>,
    pub attestation_digest: H256,
    pub attestation_block_number: u64,
    pub upper_attestation_digest: H256,
    pub upper_attestation_block_number: u64,
}

impl TestScenario {
    /// Creates a valid test scenario where everything should verify successfully
    /// Following consensus rule: continuity chain MUST go forward from attestation
    pub fn new_valid(query_height: u64, tx_index: usize) -> Self {
        // Ensure query_height is at least 5 to have proper attestation before it
        let query_height = if query_height < 5 { 5 } else { query_height };

        // Create a test block at the query height
        let test_block = create_test_block(query_height, 4);

        // Get the transaction data
        let tx_data = test_block.transactions[tx_index].data.clone();

        // Create merkle proof
        let merkle_proof = create_valid_merkle_proof_for_block(&test_block, tx_index);

        // POC pattern: attestation should ideally be at queryHeight - 2
        // This provides the lowerEndpointDigest for the continuity chain
        let attestation_block_number = query_height.saturating_sub(2);

        // POC pattern: continuity chain starts at queryHeight - 1
        let start_height = query_height.saturating_sub(1);

        // Upper attestation should be a few blocks after the query
        // This ensures the continuity chain ends at a consensus point
        let upper_attestation_block_number = query_height + 3;

        // Build continuity chain from start_height to upper_attestation_block_number
        let block_count = (upper_attestation_block_number - start_height + 1) as usize;
        let continuity_blocks =
            create_deterministic_continuity_chain(start_height, block_count, &[test_block]);

        // The lower attestation digest is the prev_digest of the first continuity block
        // This represents the digest AT the lower attestation block
        let attestation_digest = continuity_blocks[0].prev_digest;

        // The upper attestation digest is the digest of the last continuity block
        // This represents the digest AT the upper attestation block
        let upper_attestation_digest = continuity_blocks.last().unwrap().digest;

        TestScenario {
            chain_key: 1,
            height: query_height,
            tx_data,
            merkle_proof,
            continuity_blocks,
            attestation_digest,
            attestation_block_number,
            upper_attestation_digest,
            upper_attestation_block_number,
        }
    }

    /// Creates a test scenario with an invalid merkle proof
    pub fn new_with_invalid_merkle_proof(query_height: u64) -> Self {
        let mut scenario = Self::new_valid(query_height, 0);
        // Corrupt the merkle proof
        scenario.merkle_proof.siblings.push(MerkleProofEntry {
            hash: H256::random(),
            is_left: false,
        });
        scenario
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_tx_data() {
        let data1 = create_deterministic_tx_data(0);
        let data2 = create_deterministic_tx_data(0);
        assert_eq!(data1, data2, "Same index should produce same data");

        let data3 = create_deterministic_tx_data(1);
        assert_ne!(
            data1, data3,
            "Different index should produce different data"
        );
    }

    #[test]
    fn test_merkle_proof_generation() {
        let block = create_test_block(100, 4);
        let proof = create_valid_merkle_proof_for_block(&block, 0);

        // The proof.root should match the block's merkle_root
        assert_eq!(
            proof.root, block.merkle_root,
            "Proof root should match block merkle root"
        );

        // Verify using the proper verification with prefixes (matching KeccakMerkleTree)
        let mut prefixed_leaf = vec![0u8; block.transactions[0].data.len() + 1];
        prefixed_leaf[0] = 0x00; // LEAF_HASH_PREPEND_VALUE
        prefixed_leaf[1..].copy_from_slice(&block.transactions[0].data);
        let mut current_hash = H256::from(keccak_256(&prefixed_leaf));

        for sibling in &proof.siblings {
            let mut hash_input = vec![0x01]; // INNER_HASH_PREPEND_VALUE
            if sibling.is_left {
                // Sibling is on the left, current hash on the right
                hash_input.extend_from_slice(sibling.hash.as_bytes());
                hash_input.extend_from_slice(current_hash.as_bytes());
            } else {
                // Current hash on the left, sibling on the right
                hash_input.extend_from_slice(current_hash.as_bytes());
                hash_input.extend_from_slice(sibling.hash.as_bytes());
            }
            current_hash = H256::from(keccak_256(&hash_input));
        }

        // The computed hash should match the proof root
        assert_eq!(current_hash, proof.root, "Merkle proof should be valid");
    }
}
