//! Native token transfer module
//!
//! This module handles native token transfers on Ethereum-compatible chains
//! using the Alloy library for transaction building and execution.

use alloy::{
    network::{EthereumWallet, TransactionBuilder},
    primitives::{Address, FixedBytes, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::info;

/// Configuration for a native token transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferConfig {
    pub to_address: String,
    pub amount: U256,
    pub gas_price: Option<U256>,
    pub gas_limit: Option<U256>,
}

/// Result of a native token transfer
#[derive(Debug, Clone)]
pub struct TransferResult {
    pub tx_hash: FixedBytes<32>,
    pub block_number: u64,
}

/// Native token transfer executor
pub struct TransferExecutor {
    eth_rpc_url: String,
    eth_private_key: String,
    signer_address: Address,
    chain_id: u64,
}

impl TransferExecutor {
    /// Create a new transfer executor
    pub async fn new(eth_rpc_url: &str, eth_private_key: &str) -> Result<Self> {
        info!("Creating TransferExecutor with RPC URL: {}", eth_rpc_url);
        let signer =
            PrivateKeySigner::from_str(eth_private_key).context("Failed to parse private key")?;

        let signer_address = signer.address();
        info!("Signer address: {}", signer_address);
        let wallet = EthereumWallet::from(signer);

        info!("Building provider...");
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .on_http(eth_rpc_url.parse()?);

        info!("Getting chain ID...");
        let chain_id = provider.get_chain_id().await?;
        info!("Chain ID: {}", chain_id);

        Ok(Self {
            eth_rpc_url: eth_rpc_url.to_string(),
            eth_private_key: eth_private_key.to_string(),
            signer_address,
            chain_id,
        })
    }

    /// Execute a single native token transfer
    pub async fn execute_transfer(&self, config: TransferConfig) -> Result<TransferResult> {
        info!("Executing native token transfer");
        info!(
            "Transfer details: to={}, amount={}",
            config.to_address, config.amount
        );

        // Re-create provider for this transfer
        info!("Re-creating provider for transfer...");
        let signer = PrivateKeySigner::from_str(&self.eth_private_key)?;
        let wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .on_http(self.eth_rpc_url.parse()?);

        let to_address =
            Address::from_str(&config.to_address).context("Invalid recipient address")?;

        // Build transaction request
        info!("Building transaction request...");
        let mut tx = TransactionRequest::default()
            .with_to(to_address)
            .with_value(config.amount)
            .with_from(self.signer_address)
            .with_chain_id(self.chain_id);

        // Apply optional gas settings
        if let Some(gas_price) = config.gas_price {
            tx = tx.with_gas_price(gas_price.to::<u128>());
        }

        if let Some(gas_limit) = config.gas_limit {
            tx = tx.with_gas_limit(gas_limit.to::<u64>());
        }

        info!(
            "Sending {} wei from {} to {}",
            config.amount, self.signer_address, config.to_address
        );

        // Send transaction
        info!("Sending transaction...");
        let pending_tx = provider
            .send_transaction(tx)
            .await
            .context("Failed to send transaction")?;

        let tx_hash = *pending_tx.tx_hash();
        info!("Transaction sent: 0x{:x}", tx_hash);

        // Wait for confirmation
        info!("Waiting for transaction confirmation...");
        let receipt = pending_tx
            .get_receipt()
            .await
            .context("Failed to get transaction receipt")?;

        let block_number = receipt
            .block_number
            .ok_or_else(|| anyhow::anyhow!("Block number not found in receipt"))?;

        info!("Transaction confirmed in block {}", block_number);

        Ok(TransferResult {
            tx_hash,
            block_number,
        })
    }

    /// Execute multiple transfers sequentially
    pub async fn execute_batch_transfers(
        &self,
        configs: Vec<TransferConfig>,
    ) -> Result<Vec<TransferResult>> {
        info!("Executing {} native token transfers", configs.len());

        let mut results = Vec::new();
        let total = configs.len();

        for (i, config) in configs.into_iter().enumerate() {
            info!("Processing transfer {}/{}", i + 1, total);

            let result = self.execute_transfer(config).await?;
            results.push(result);

            // Small delay between transfers to avoid nonce issues
            if i < total - 1 {
                sleep(Duration::from_secs(2)).await;
            }
        }

        info!("All transfers completed successfully");
        Ok(results)
    }
}

/// Helper function to create a transfer configuration
pub fn create_transfer_config(to_address: String, amount_wei: u64) -> TransferConfig {
    TransferConfig {
        to_address,
        amount: U256::from(amount_wei),
        gas_price: None,
        gas_limit: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_transfer_config() {
        let config = create_transfer_config(
            "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb8".to_string(),
            1000000000000000000, // 1 ETH in wei
        );

        assert_eq!(
            config.to_address,
            "0x742d35Cc6634C0532925a3b844Bc9e7595f0bEb8"
        );
        assert_eq!(config.amount, U256::from(1000000000000000000u64));
    }
}
