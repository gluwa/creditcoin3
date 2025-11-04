//! Query verification module
//!
//! This module handles the verification of queries against the native query verifier
//! precompile and displays the results.

use anyhow::Result;
use attestor_primitives::block::Block;
use attestor_primitives::{Query, ResultSegment};
use eth::{evm, Client};
use mmr::query_proof::QueryMerkleProof;

/// Configuration for query verification
pub struct VerificationConfig {
    pub cc3_rpc_url: String,
    pub cc3_evm_private_key: String,
}

/// Result of query verification
pub struct VerificationResult {
    pub success: bool,
    pub segments: Vec<ResultSegment>,
    pub gas_estimate: Option<u64>,
    pub merkle_siblings_count: usize,
    pub continuity_blocks_count: usize,
    pub tx_data_size: usize,
}

/// Verify a query using the native query verifier precompile
pub async fn verify_query(
    config: &VerificationConfig,
    query: &Query,
    tx_data: &[u8],
    merkle_proof: QueryMerkleProof,
    continuity_blocks: Vec<Block>,
) -> Result<VerificationResult> {
    // Initialize the Ethereum client for Creditcoin3
    let eth_client = Client::new(&config.cc3_rpc_url, Some(&config.cc3_evm_private_key)).await?;
    let verifier = evm::native_query_verifier::NativeQueryVerifierContract::new(&eth_client);

    // Try to estimate gas (optional, may fail if continuity is not ready)
    let gas_estimate = verifier
        .estimate_gas(
            query,
            tx_data,
            merkle_proof.clone(),
            continuity_blocks.clone(),
        )
        .await
        .ok();

    // Store context for gas analysis
    let merkle_siblings_count = merkle_proof.siblings.len();
    let continuity_blocks_count = continuity_blocks.len();
    let tx_data_size = tx_data.len();

    // Call the verifier
    match verifier
        .verify_query(query, tx_data, merkle_proof, continuity_blocks)
        .await
    {
        Ok(result) => Ok(VerificationResult {
            success: true,
            segments: result.result_segments,
            gas_estimate,
            merkle_siblings_count,
            continuity_blocks_count,
            tx_data_size,
        }),
        Err(e) => {
            // Log the error but return a failed result
            eprintln!("Verification failed: {e:?}");
            Ok(VerificationResult {
                success: false,
                segments: vec![],
                gas_estimate,
                merkle_siblings_count,
                continuity_blocks_count,
                tx_data_size,
            })
        }
    }
}

/// Display verification results
pub fn display_results(query: &Query, result: &VerificationResult) {
    // Display gas estimate if available
    if let Some(gas) = result.gas_estimate {
        println!("\n⛽ Gas Estimation:");
        println!("   Total gas units: {gas}");
        println!("   ─────────────────────────────────────");

        // Cost estimates at various gas prices
        let gas_prices = [
            (10, "10 gwei (low)"),
            (20, "20 gwei (avg)"),
            (50, "50 gwei (high)"),
            (100, "100 gwei (very high)"),
        ];

        println!("   Estimated costs:");
        for (gwei, label) in gas_prices {
            let eth_cost = gas as f64 * gwei as f64 / 1_000_000_000.0;
            println!("     {} ETH at {}", format_eth(eth_cost), label);
        }

        println!("\n   Gas cost factors:");
        println!(
            "     • Merkle proof verification ({} siblings)",
            result.merkle_siblings_count
        );
        println!(
            "     • Continuity chain validation ({} blocks)",
            result.continuity_blocks_count
        );
        println!(
            "     • Data extraction from transaction ({} bytes)",
            result.tx_data_size
        );
        println!(
            "     • Result segment processing ({} segments)",
            query.layout_segments.len()
        );

        println!("\n   This query parameters:");
        println!("     • Chain ID: {}", query.chain_id);
        println!("     • Block height: {}", query.height);

        println!("     • Layout segments: {}", query.layout_segments.len());

        println!("\n   Comparison with Solidity smart contract:");
        println!("     Native Precompile (0x0FD2): {gas} gas");
        let estimated_solidity_gas = estimate_solidity_equivalent(
            result.merkle_siblings_count,
            result.continuity_blocks_count,
            result.tx_data_size,
            query.layout_segments.len(),
        );
        println!("     Solidity Contract (est.): ~{estimated_solidity_gas} gas");
        let savings = ((estimated_solidity_gas as f64 - gas as f64) / estimated_solidity_gas as f64
            * 100.0) as i32;
        if savings > 0 {
            println!("     Savings: ~{savings}% lower cost");
        }

        println!("\n   Note: Native precompile provides optimized");
        println!("         verification with reduced gas costs");
    }

    if result.success {
        println!("\n✅ Verification successful!");
        display_extracted_data(&result.segments);
    } else {
        println!("\n❌ Verification failed");
        println!("Query ID: {:?}", query.id());
        println!("Note: This may be due to missing continuity chain data");
    }
}

/// Format ETH value for display
fn format_eth(eth: f64) -> String {
    if eth < 0.001 {
        format!("{eth:.9}")
    } else if eth < 0.01 {
        format!("{eth:.6}")
    } else {
        format!("{eth:.4}")
    }
}

/// Estimate gas cost for equivalent Solidity smart contract implementation
fn estimate_solidity_equivalent(
    merkle_siblings: usize,
    continuity_blocks: usize,
    tx_data_size: usize,
    layout_segments: usize,
) -> u64 {
    // Rough estimates based on typical Solidity operations:
    // - Storage reads: ~2100 gas each
    // - Hash operations (keccak256): ~30 gas + 6 gas per word
    // - Memory operations: ~3 gas per word
    // - CALL overhead: ~700 gas

    let base_cost = 21000u64; // Transaction base cost

    // Merkle proof verification in Solidity
    // Each sibling requires a hash operation (keccak256)
    let merkle_cost = merkle_siblings as u64 * 50; // ~50 gas per hash with memory ops

    // Continuity chain validation
    // Each block requires hash computation and comparisons
    let continuity_cost = continuity_blocks as u64 * 100; // ~100 gas per block validation

    // Data extraction and ABI decoding in Solidity
    // More expensive due to calldata processing and memory allocation
    let data_extraction_cost = (tx_data_size as u64 / 32) * 10; // ~10 gas per word

    // Result segment processing
    let segment_cost = layout_segments as u64 * 50; // ~50 gas per segment processing

    // Solidity overhead (stack operations, jumps, etc.)
    let overhead = 5000;

    base_cost + merkle_cost + continuity_cost + data_extraction_cost + segment_cost + overhead
}

/// Display extracted data segments
fn display_extracted_data(segments: &[ResultSegment]) {
    if segments.is_empty() {
        println!("No data segments extracted");
        return;
    }

    println!("Result segments count: {}", segments.len());
    for (i, segment) in segments.iter().enumerate() {
        println!(
            "  Segment {}: offset={}, bytes=0x{}",
            i,
            segment.offset,
            hex::encode(segment.bytes.as_bytes())
        );

        // Try to interpret common patterns
        let bytes = segment.bytes.as_bytes();
        // Check if it looks like an address (20 bytes with 12 bytes padding)
        if bytes[0..12] == [0u8; 12] {
            let address = &bytes[12..32];
            if address.iter().any(|&b| b != 0) {
                println!("    (Possible address: 0x{})", hex::encode(address));
            }
        }
        // Check if it looks like a uint256
        else if bytes[0..16] == [0u8; 16] {
            let value = &bytes[16..32];
            if value.iter().any(|&b| b != 0) {
                println!("    (Possible value: 0x{})", hex::encode(value));
            }
        }
    }
}
