//! Workflow orchestration module
//!
//! This module provides high-level workflows that combine transfers, attestation waiting,
//! and query execution into cohesive operations.

use crate::batch_verification::execute_batch_query;
use crate::native_transfer::{TransferConfig, TransferExecutor, TransferResult};
use crate::NativeQueryParams;
use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{info, warn};

/// Configuration for transfer and query workflow
#[derive(Debug, Clone)]
pub struct TransferQueryConfig {
    /// Whether to wait for attestation before querying
    pub wait_for_attestation: bool,
    /// Maximum time to wait for attestation
    pub attestation_timeout: Duration,
    /// Whether to automatically query after attestation
    pub auto_query: bool,
    /// Whether to send query as transaction (costs gas)
    pub send_tx: bool,
}

impl Default for TransferQueryConfig {
    fn default() -> Self {
        Self {
            wait_for_attestation: true,
            attestation_timeout: Duration::from_secs(300),
            auto_query: true,
            send_tx: false,
        }
    }
}

/// Result of a transfer and query workflow
#[derive(Debug)]
pub struct TransferQueryResult {
    /// The transfer result
    pub transfer: TransferResult,
    /// Block number where attestation was confirmed (if waited)
    pub attestation_block: Option<u64>,
    /// Query execution result (if auto-query was enabled)
    pub query_result: Option<QueryResult>,
}

/// Result of query execution
#[derive(Debug)]
pub struct QueryResult {
    pub query_id: String,
    pub success: bool,
}

/// Execute a single transfer with optional attestation waiting and query
pub async fn execute_transfer_with_query(
    eth_rpc_url: &str,
    eth_private_key: &str,
    cc3_rpc_url: &str,
    cc3_evm_private_key: &str,
    transfer_config: TransferConfig,
    workflow_config: TransferQueryConfig,
    chain_key: u64,
) -> Result<TransferQueryResult> {
    info!("Starting transfer and query workflow");
    info!("ETH RPC URL: {}", eth_rpc_url);
    info!(
        "Transfer config: to={}, amount={}",
        transfer_config.to_address, transfer_config.amount
    );

    // Step 1: Execute the transfer
    info!("Creating transfer executor...");
    let executor = TransferExecutor::new(eth_rpc_url, eth_private_key).await?;
    info!("Executing transfer...");
    let transfer_result = executor.execute_transfer(transfer_config).await?;

    info!(
        "Transfer completed: 0x{:x} in block {}",
        transfer_result.tx_hash, transfer_result.block_number
    );

    let mut result = TransferQueryResult {
        transfer: transfer_result.clone(),
        attestation_block: None,
        query_result: None,
    };

    // Step 2: Wait for attestation if requested
    if workflow_config.wait_for_attestation {
        info!(
            "Waiting for block {} to be attested...",
            transfer_result.block_number
        );

        let attestation_block = wait_for_attestation_with_chain_key(
            cc3_rpc_url,
            chain_key,
            transfer_result.block_number,
            workflow_config.attestation_timeout,
        )
        .await
        .context("Failed to wait for attestation")?;

        result.attestation_block = Some(attestation_block);
        info!(
            "Block {} attested at Creditcoin block {}",
            transfer_result.block_number, attestation_block
        );
    }

    // Step 3: Execute query if requested
    if workflow_config.auto_query {
        if !workflow_config.wait_for_attestation {
            warn!("Auto-query requested but attestation waiting disabled. Query may fail if block not yet attested.");
        }

        info!("Executing query for transfer transaction");

        // Call the native query submission
        // This would integrate with the existing query system
        let query_result = execute_transfer_query(
            cc3_rpc_url,
            cc3_evm_private_key,
            eth_rpc_url,
            &format!("0x{:x}", transfer_result.tx_hash),
            transfer_result.block_number,
            chain_key,
            workflow_config.send_tx,
        )
        .await?;

        result.query_result = Some(query_result);
    }

    Ok(result)
}

/// Execute batch transfers with optional attestation waiting and batch query
pub async fn execute_batch_transfers_with_query(
    eth_rpc_url: &str,
    eth_private_key: &str,
    cc3_rpc_url: &str,
    cc3_evm_private_key: &str,
    transfer_configs: Vec<TransferConfig>,
    workflow_config: TransferQueryConfig,
    chain_key: u64,
) -> Result<Vec<TransferQueryResult>> {
    info!(
        "Starting batch transfer workflow for {} transfers",
        transfer_configs.len()
    );

    let executor = TransferExecutor::new(eth_rpc_url, eth_private_key).await?;
    let mut results = Vec::new();

    // Step 1: Execute all transfers
    let transfer_results = executor.execute_batch_transfers(transfer_configs).await?;

    info!("All {} transfers completed", transfer_results.len());

    // Step 2: Wait for attestations if requested
    for transfer in transfer_results {
        let mut result = TransferQueryResult {
            transfer: transfer.clone(),
            attestation_block: None,
            query_result: None,
        };

        if workflow_config.wait_for_attestation {
            info!(
                "Waiting for block {} to be attested...",
                transfer.block_number
            );

            match wait_for_attestation_with_chain_key(
                cc3_rpc_url,
                chain_key,
                transfer.block_number,
                workflow_config.attestation_timeout,
            )
            .await
            {
                Ok(attestation_block) => {
                    result.attestation_block = Some(attestation_block);
                    info!(
                        "Block {} attested at Creditcoin block {}",
                        transfer.block_number, attestation_block
                    );
                }
                Err(e) => {
                    warn!(
                        "Failed to wait for attestation of block {}: {}",
                        transfer.block_number, e
                    );
                    // Continue with other transfers
                }
            }
        }

        results.push(result);
    }

    // Step 3: Execute batch query if requested
    if workflow_config.auto_query && !results.is_empty() {
        info!("Executing batch query for {} transfers", results.len());

        let tx_hashes: Vec<String> = results
            .iter()
            .map(|r| format!("0x{:x}", r.transfer.tx_hash))
            .collect();

        let block_heights: Vec<u64> = results.iter().map(|r| r.transfer.block_number).collect();

        // Execute batch query
        execute_batch_query(
            cc3_rpc_url.to_string(),
            cc3_evm_private_key.to_string(),
            eth_rpc_url.to_string(),
            tx_hashes,
            block_heights,
            chain_key,
            workflow_config.send_tx,
        )
        .await
        .context("Failed to execute batch query")?;

        // Update results with query success
        for result in &mut results {
            result.query_result = Some(QueryResult {
                query_id: format!("0x{:x}", result.transfer.tx_hash),
                success: true,
            });
        }
    }

    Ok(results)
}

/// Execute a query for a transfer transaction
async fn execute_transfer_query(
    cc3_rpc_url: &str,
    cc3_evm_private_key: &str,
    eth_rpc_url: &str,
    tx_hash: &str,
    block_height: u64,
    chain_key: u64,
    send_tx: bool,
) -> Result<QueryResult> {
    use crate::submit_native_query;

    // Call the existing native query submission
    let params = NativeQueryParams {
        cc3_rpc_url: cc3_rpc_url.to_string(),
        cc3_evm_private_key: cc3_evm_private_key.to_string(),
        eth_rpc_url: Some(eth_rpc_url.to_string()),
        block_height: Some(block_height),
        txn_hash: Some(tx_hash.to_string()),
        data_choice: Some(3), // Native transfer data choice (3 = NativeTokenTransferData)
        chain_key,
        send_tx,
    };
    submit_native_query(params)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute query: {}", e))?;

    Ok(QueryResult {
        query_id: tx_hash.to_string(),
        success: true,
    })
}

/// Helper function to create workflow config from CLI arguments
pub fn create_workflow_config(
    wait_attestation: bool,
    auto_query: bool,
    send_tx: bool,
) -> TransferQueryConfig {
    TransferQueryConfig {
        wait_for_attestation: wait_attestation,
        attestation_timeout: Duration::from_secs(300),
        auto_query,
        send_tx,
    }
}

/// Wait for attestation with explicit chain key
async fn wait_for_attestation_with_chain_key(
    cc3_rpc_url: &str,
    chain_key: u64,
    block_number: u64,
    max_wait: Duration,
) -> Result<u64> {
    use crate::attestation::AttestationConfig;
    use crate::attestation::AttestationMonitor;

    let config = AttestationConfig {
        max_wait_time: max_wait,
        ..Default::default()
    };

    let monitor = AttestationMonitor::new(cc3_rpc_url, config).await?;
    let result = monitor
        .wait_for_block_attestation_with_chain_key(chain_key, block_number)
        .await?;

    Ok(result.attested_block)
}

/// Generate test transfers for CI mode
pub fn generate_ci_test_transfers(count: usize, base_amount: u64) -> Vec<TransferConfig> {
    (0..count)
        .map(|i| TransferConfig {
            to_address: format!("0x{:040x}", 0x1000 + i),
            amount: alloy::primitives::U256::from(base_amount + (i as u64 * 1000)),
            gas_price: None,
            gas_limit: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_config_default() {
        let config = TransferQueryConfig::default();
        assert!(config.wait_for_attestation);
        assert_eq!(config.attestation_timeout, Duration::from_secs(300));
        assert!(config.auto_query);
        assert!(!config.send_tx);
    }

    #[test]
    fn test_generate_ci_test_transfers() {
        let transfers = generate_ci_test_transfers(3, 1000);
        assert_eq!(transfers.len(), 3);
        assert_eq!(transfers[0].amount, alloy::primitives::U256::from(1000));
        assert_eq!(transfers[1].amount, alloy::primitives::U256::from(2000));
        assert_eq!(transfers[2].amount, alloy::primitives::U256::from(3000));
    }
}
