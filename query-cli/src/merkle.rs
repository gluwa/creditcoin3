//! Merkle proof generation module
//!
//! This module handles the generation and preparation of Merkle proofs
//! for transaction verification using the simple, Ethereum-compatible approach.

use ::merkle::TransactionMerkleProof;
use anyhow::Result;
use eth::OrderedBlock;
use utils::block_item_traits::BlockItem;

/// Generate a Merkle proof for a transaction in a block
pub fn generate_merkle_proof(
    block: &OrderedBlock,
    tx_index: usize,
) -> Result<TransactionMerkleProof> {
    // Build the simple Merkle tree using the eth helper function
    let tree = eth::simple_merkle_tree(block);

    // Generate proof for the specified transaction
    let proof = tree.generate_proof(tx_index).unwrap();

    Ok(TransactionMerkleProof::new(tree.root(), proof.siblings))
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
