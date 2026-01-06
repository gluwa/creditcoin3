//! Query verification module
//!
//! This module handles the verification of queries against the native query verifier
//! precompile and displays the results.

use anyhow::Result;
use attestor_primitives::block::{Block, ContinuityProof};
use eth::{evm, Client};
use merkle::TransactionMerkleProof;

/// Configuration for query verification
#[derive(Debug, Clone)]
pub struct VerificationConfig {
    pub cc3_rpc_url: String,
    pub cc3_evm_private_key: String,
    pub eth_rpc_url: String,
    pub chain_key: u64,
}

/// Result of query verification
pub struct VerificationResult {
    pub success: bool,
    pub gas_estimate: Option<u64>,
    pub merkle_siblings_count: usize,
    pub continuity_blocks_count: usize,
    pub tx_data_size: usize,
}

/// Verify a query using the native query verifier precompile
pub async fn verify_query(
    config: &VerificationConfig,
    chain_key: u64,
    height: u64,
    tx_data: &[u8],
    merkle_proof: TransactionMerkleProof,
    continuity_blocks: Vec<Block>,
    send_tx: bool,
) -> Result<VerificationResult> {
    // Initialize the Ethereum client for Creditcoin3
    let eth_client = Client::new(&config.cc3_rpc_url, Some(&config.cc3_evm_private_key)).await?;
    let verifier = evm::block_prover::BlockProver::new(&eth_client);

    // Convert Vec<Block> to ContinuityProof for the optimized API
    let continuity_proof = ContinuityProof::from_blocks(continuity_blocks.clone());
    tracing::debug!("Continuity proof: {:?}", continuity_proof);

    // Try to estimate gas (optional, may fail if continuity is not ready)
    let gas_estimate = verifier
        .estimate_gas(
            chain_key,
            height,
            tx_data,
            merkle_proof.clone(),
            continuity_proof.clone(),
        )
        .await
        .ok();

    // Store context for gas analysis
    let merkle_siblings_count = merkle_proof.siblings.len();
    let continuity_blocks_count = continuity_blocks.len();
    let tx_data_size = tx_data.len();

    // Call the verifier (as transaction if requested to emit events)
    let verification_result = if send_tx {
        verifier
            .verify_and_emit(
                chain_key,
                height,
                tx_data,
                merkle_proof,
                continuity_proof.clone(),
            )
            .await
    } else {
        verifier
            .verify(chain_key, height, tx_data, merkle_proof, continuity_proof)
            .await
    };

    match verification_result {
        Ok(success) => Ok(VerificationResult {
            success,
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
                gas_estimate,
                merkle_siblings_count,
                continuity_blocks_count,
                tx_data_size,
            })
        }
    }
}

/// Display verification results
pub fn display_results(chain_key: u64, height: u64, result: &VerificationResult) {
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

        println!("\n   This query parameters:");
        println!("     • Chain ID: {chain_key}");
        println!("     • Block height: {height}");

        println!("\n   Comparison with Solidity smart contract:");
        println!("     Native Precompile (0x0FD2): {gas} gas");
    }

    if result.success {
        println!("\n✅ Verification successful!");
    } else {
        println!("\n❌ Verification failed");
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
