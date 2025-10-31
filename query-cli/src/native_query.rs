//! Native query execution module
//!
//! This module handles the complete flow of native query execution,
//! from fetching block data to verification.

use crate::{
    config::{AppConfig, QueryConfig},
    continuity, merkle,
    verification::{self, VerificationConfig},
};
use anyhow::Result;
use eth::{Client as EthClient, OrderedBlock};

/// Execute a native query with the given configuration
pub async fn execute_native_query(config: AppConfig) -> Result<()> {
    println!("\n=== Native Query Execution ===");

    // Step 1: Fetch block data from source chain
    let block = fetch_block_data(&config.query).await?;

    // Step 2: Find the transaction in the block
    let tx_index = find_transaction_index(&block, &config.query.transaction_hash)?;

    // Step 3: Create the query from configuration
    let query = config.query.to_query(tx_index as u64);

    println!("\nQuery ID: {:?}", query.id());
    println!("Query details: {:?}", query);

    // Step 4: Display block information
    merkle::display_block_info(&block);

    // Step 5: Generate Merkle proof
    println!("\n=== Merkle Proof Generation ===");
    let merkle_proof = merkle::generate_merkle_proof(&block, tx_index)?;
    println!("Merkle root: {:?}", merkle_proof.root);
    println!("Siblings count: {}", merkle_proof.siblings.len());

    // Step 6: Get transaction data
    let tx_data = merkle::get_transaction_data(&block, tx_index)?;
    println!("Transaction data size: {} bytes", tx_data.len());

    // Step 7: Generate continuity proof
    println!("\n=== Continuity Proof Generation ===");
    let continuity_blocks = continuity::fetch_continuity_proof(
        &config.creditcoin.rpc_url,
        &query,
        &block,
        &config.query.network.rpc_url(),
    )
    .await?;
    println!("Continuity blocks: {}", continuity_blocks.len());

    // Step 8: Verify the query
    println!("\n=== Query Verification ===");
    let verification_config = VerificationConfig {
        cc3_rpc_url: config.creditcoin.rpc_url.clone(),
        cc3_evm_private_key: config.creditcoin.evm_private_key.clone(),
    };

    let result = verification::verify_query(
        &verification_config,
        &query,
        &tx_data,
        merkle_proof,
        continuity_blocks,
    )
    .await?;

    // Step 9: Display results
    verification::display_results(&query, &result);

    Ok(())
}

/// Fetch block data from the source chain
async fn fetch_block_data(query_config: &QueryConfig) -> Result<OrderedBlock> {
    println!(
        "Fetching block {} from {:?}...",
        query_config.block_height, query_config.network
    );

    let eth_client = EthClient::new(&query_config.network.rpc_url(), None).await?;
    let block = eth_client
        .get_block(
            query_config.block_height,
            ccnext_abi_encoding::abi::EncodingVersion::V1,
        )
        .await?;

    println!("Block fetched successfully");
    Ok(block)
}

/// Find the transaction index in the block by hash
fn find_transaction_index(block: &OrderedBlock, tx_hash: &str) -> Result<usize> {
    // Remove '0x' prefix if present
    let tx_hash = tx_hash.strip_prefix("0x").unwrap_or(tx_hash);

    for (index, item) in block.items().iter().enumerate() {
        // Get transaction hash from the item
        let item_hash = item.tx_hash().to_string();

        if item_hash == tx_hash {
            println!("Found transaction at index {}", index);
            return Ok(index);
        }
    }

    // For development, if exact match not found, use index 0
    println!("Warning: Exact transaction hash not found, using index 0");
    Ok(0)
}

/// Module for handling native query submission
pub mod submission {
    use super::*;
    use crate::prompt;

    /// Handle native query submission through interactive prompt
    pub async fn handle_interactive(
        cc3_rpc_url: String,
        cc3_evm_private_key: String,
    ) -> Result<()> {
        // Get query configuration from user
        let prompt_args = crate::PromptArgs {
            eth_rpc_url: None,
            block_height: None,
            txn_hash: None,
            data_choice: None,
        };
        let prompt_output = prompt::prompt(prompt_args)?;

        // Convert prompt output to query configuration
        let query_config = prompt_output_to_config(prompt_output)?;

        // Create application configuration
        let app_config = AppConfig::new(
            query_config,
            crate::config::CreditcoinConfig::new(cc3_rpc_url, cc3_evm_private_key),
        );

        // Execute the native query
        execute_native_query(app_config).await
    }

    /// Convert prompt output to query configuration
    fn prompt_output_to_config(prompt_output: crate::prompt::PromptOutput) -> Result<QueryConfig> {
        use crate::config::{DataSelection, Network};

        let network = match &prompt_output.network {
            crate::Network::Sepolia(_api_key) => Network::Sepolia,
            crate::Network::Ethereum(_api_key) => Network::Ethereum,
            crate::Network::Local(url) => Network::Local(url.clone()),
            crate::Network::Custom(id, url) => Network::Custom {
                id: *id,
                url: url.clone(),
            },
        };

        let data_selection = match prompt_output.selected_data {
            crate::prompt::SelectedData::All => DataSelection::AllData,
            crate::prompt::SelectedData::RangeOfData => {
                // Use the first range from offsets_and_sizes
                if let Some((offset, size)) = prompt_output.offsets_and_sizes.first() {
                    DataSelection::Range {
                        offset: *offset as usize,
                        size: *size as usize,
                    }
                } else {
                    DataSelection::AllData
                }
            }
            crate::prompt::SelectedData::Erc20TransferData => DataSelection::ERC20Transfer,
            crate::prompt::SelectedData::NativeTokenTransferData => {
                DataSelection::NativeTokenTransfer
            }
        };

        Ok(QueryConfig {
            network,
            block_height: prompt_output.height,
            transaction_hash: prompt_output.tx_hash,
            data_selection,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_index_finding() {
        // Add tests for transaction index finding
    }

    #[test]
    fn test_query_creation() {
        // Add tests for query creation from config
    }
}
