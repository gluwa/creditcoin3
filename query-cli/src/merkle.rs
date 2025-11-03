//! Merkle proof generation module
//!
//! This module handles the generation and preparation of Merkle proofs
//! for transaction verification using the simple, Ethereum-compatible approach.

use anyhow::Result;
use eth::{evm, OrderedBlock};
use utils::block_item_traits::BlockItem;
use utils::simple_merkle::proof_to_precompile_format;

/// Generate a Merkle proof for a transaction in a block
pub fn generate_merkle_proof(
    block: &OrderedBlock,
    tx_index: usize,
) -> Result<evm::native_query_verifier::MerkleProof> {
    // Build the simple Merkle tree using the eth helper function
    let tree = eth::simple_merkle_tree(block);

    // Generate proof for the specified transaction
    let proof = tree.generate_proof(tx_index);

    // Convert to precompile format (flat array with placeholders)
    let siblings = proof_to_precompile_format(&proof, tx_index);

    Ok(evm::native_query_verifier::MerkleProof {
        root: tree.root(),
        siblings,
    })
}

/// Get transaction data from the block
pub fn get_transaction_data(block: &OrderedBlock, tx_index: usize) -> Result<Vec<u8>> {
    let tx = block
        .items()
        .get(tx_index)
        .ok_or_else(|| anyhow::anyhow!("Transaction index {} not found in block", tx_index))?;

    // Use to_bytes() to get the same data that was used to build the Merkle tree
    // This includes the BlockItemIdentifier (16 bytes: block_number + index)
    Ok(tx.to_bytes())
}

/// Display block structure information
pub fn display_block_info(block: &OrderedBlock) {
    println!("\n=== Block Structure ===");
    println!("Block number: {}", block.number());
    println!("Total transactions in block: {}", block.items().len());

    for (idx, item) in block.items().iter().take(5).enumerate() {
        let tx_bytes = item.to_bytes();
        println!("  Transaction {}: {} bytes", idx, tx_bytes.len());

        if tx_bytes.len() >= 32 {
            println!(
                "    First bytes: {:?}",
                hex::encode(&tx_bytes[..32.min(tx_bytes.len())])
            );
        }
    }

    if block.items().len() > 5 {
        println!("  ... and {} more transactions", block.items().len() - 5);
    }
}
