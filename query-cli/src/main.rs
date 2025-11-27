use anyhow::Result;
use clap::{Parser, Subcommand};
use std::error::Error;
use utils::block_item_traits::BlockItem;

mod attestation;
mod batch_verification;
mod merkle;
mod native_transfer;
mod prompt;
mod query_builder;
mod verification;
mod workflow;

use crate::prompt::prompt as prompt_user;
use eth::Client;

// Configuration structs to group related parameters
#[derive(Debug, Clone)]
struct ConnectionConfig {
    cc3_rpc_url: String,
    cc3_evm_private_key: String,
    eth_rpc_url: String,
    eth_private_key: String,
}

#[derive(Debug, Clone)]
struct QueryParams {
    chain_key: u64,
    wait_attestation: bool,
    auto_query: bool,
    send_tx: bool,
    ci_mode: bool,
}

#[derive(Debug, Clone)]
pub struct NativeQueryParams {
    pub cc3_rpc_url: String,
    pub cc3_evm_private_key: String,
    pub eth_rpc_url: Option<String>,
    pub block_height: Option<u64>,
    pub txn_hash: Option<String>,
    pub data_choice: Option<u64>,
    pub chain_key: u64,
    pub send_tx: bool,
}

#[cfg(test)]
mod tests;

#[derive(Parser, Debug, Clone)]
#[command(name = "query-cli")]
pub struct QueryCli {
    #[arg(long, required = true, default_value = "http://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_evm_private_key: String,

    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Commands {
    /// Verify query through the native precompile (direct verification)
    Verify {
        #[arg(long)]
        eth_rpc_url: Option<String>,

        #[arg(long)]
        block_height: Option<u64>,

        #[arg(long)]
        txn_hash: Option<String>,

        #[arg(long)]
        data_choice: Option<u64>,

        /// Chain key for attestation (Creditcoin3 chain identifier)
        #[arg(long, default_value = "2")]
        chain_key: u64,

        /// Send as transaction to emit events (costs gas)
        #[arg(long)]
        send_tx: bool,
    },

    /// Execute a native token transfer and query it once attested
    Transfer {
        #[arg(long)]
        eth_rpc_url: String,

        #[arg(long)]
        eth_private_key: String,

        #[arg(long)]
        to_address: String,

        #[arg(long)]
        amount_wei: u64,

        /// Chain key for attestation monitoring (Creditcoin3 chain identifier)
        #[arg(long)]
        chain_key: u64,

        /// Wait for the transfer to be attested before querying
        #[arg(long, default_value = "true")]
        wait_attestation: bool,

        /// Automatically query the transfer once attested
        #[arg(long, default_value = "true")]
        auto_query: bool,

        /// Send query as transaction to emit events (costs gas)
        #[arg(long)]
        send_tx: bool,
    },

    /// Execute multiple native token transfers and batch query them
    BatchTransfer {
        #[arg(long)]
        eth_rpc_url: String,

        #[arg(long)]
        eth_private_key: String,

        /// Number of transfers to execute
        #[arg(long, default_value = "3")]
        count: usize,

        /// Base amount in wei (each transfer increments by 1000)
        #[arg(long, default_value = "1000000000000000")]
        base_amount: u64,

        /// Chain key for attestation monitoring (Creditcoin3 chain identifier)
        #[arg(long)]
        chain_key: u64,

        /// Wait for all transfers to be attested before querying
        #[arg(long, default_value = "true")]
        wait_attestation: bool,

        /// Automatically batch query all transfers once attested
        #[arg(long, default_value = "true")]
        auto_query: bool,

        /// Send query as transaction to emit events (costs gas)
        #[arg(long)]
        send_tx: bool,

        /// Use test addresses for CI automation
        #[arg(long)]
        ci_mode: bool,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = QueryCli::parse();

    // enable tracing debug logs if verbose flag is set
    let env_filter = if args.verbose {
        println!("debug mode enabled!");
        "debug"
    } else {
        "prover=info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(true)
        .with_env_filter(env_filter)
        .try_init();

    match args.command {
        Commands::Verify {
            eth_rpc_url,
            block_height,
            txn_hash,
            data_choice,
            chain_key,
            send_tx,
        } => {
            // If all parameters are provided via CLI, use direct execution
            // Otherwise, use interactive mode
            if eth_rpc_url.is_some() && block_height.is_some() && txn_hash.is_some() {
                let params = NativeQueryParams {
                    cc3_rpc_url: args.cc3_rpc_url,
                    cc3_evm_private_key: args.cc3_evm_private_key,
                    eth_rpc_url,
                    block_height,
                    txn_hash,
                    data_choice,
                    chain_key,
                    send_tx,
                };
                submit_native_query(params).await?;
            } else {
                // Use interactive mode
                handle_interactive_query(
                    args.cc3_rpc_url,
                    args.cc3_evm_private_key,
                    chain_key,
                    send_tx,
                )
                .await?;
            }
        }
        Commands::Transfer {
            eth_rpc_url,
            eth_private_key,
            to_address,
            amount_wei,
            chain_key,
            wait_attestation,
            auto_query,
            send_tx,
        } => {
            let conn = ConnectionConfig {
                cc3_rpc_url: args.cc3_rpc_url,
                cc3_evm_private_key: args.cc3_evm_private_key,
                eth_rpc_url,
                eth_private_key,
            };
            let query = QueryParams {
                chain_key,
                wait_attestation,
                auto_query,
                send_tx,
                ci_mode: false,
            };
            handle_transfer_and_query(conn, to_address, amount_wei, query).await?;
        }
        Commands::BatchTransfer {
            eth_rpc_url,
            eth_private_key,
            count,
            base_amount,
            chain_key,
            wait_attestation,
            auto_query,
            send_tx,
            ci_mode,
        } => {
            let conn = ConnectionConfig {
                cc3_rpc_url: args.cc3_rpc_url,
                cc3_evm_private_key: args.cc3_evm_private_key,
                eth_rpc_url,
                eth_private_key,
            };
            let query = QueryParams {
                chain_key,
                wait_attestation,
                auto_query,
                send_tx,
                ci_mode,
            };
            handle_batch_transfer_and_query(conn, count, base_amount, query).await?;
        }
    }

    Ok(())
}

async fn handle_transfer_and_query(
    conn: ConnectionConfig,
    to_address: String,
    amount_wei: u64,
    query: QueryParams,
) -> Result<(), Box<dyn Error>> {
    use crate::native_transfer::create_transfer_config;
    use crate::workflow::{create_workflow_config, execute_transfer_with_query};

    println!("\n=== Executing Native Token Transfer ===");

    // Create transfer config
    let transfer_config = create_transfer_config(to_address, amount_wei);

    // Create workflow config
    let workflow_config =
        create_workflow_config(query.wait_attestation, query.auto_query, query.send_tx);

    // Execute transfer with optional attestation and query
    let result = execute_transfer_with_query(
        &conn.eth_rpc_url,
        &conn.eth_private_key,
        &conn.cc3_rpc_url,
        &conn.cc3_evm_private_key,
        transfer_config,
        workflow_config,
        query.chain_key,
    )
    .await?;

    println!("--------------------------------");
    println!("Transfer successful!");
    println!("  Transaction hash: 0x{:x}", result.transfer.tx_hash);
    println!("  Block number: {}", result.transfer.block_number);

    if let Some(attestation_block) = result.attestation_block {
        println!("  Attested at CC3 block: {attestation_block}");
    }

    if let Some(query_result) = result.query_result {
        println!("\n=== Query Result ===");
        println!("  Query ID: {}", query_result.query_id);
        println!("  Success: {}", query_result.success);
    }

    Ok(())
}

async fn handle_batch_transfer_and_query(
    conn: ConnectionConfig,
    count: usize,
    base_amount: u64,
    query: QueryParams,
) -> Result<(), Box<dyn Error>> {
    use crate::native_transfer::create_transfer_config;
    use crate::workflow::{
        create_workflow_config, execute_batch_transfers_with_query, generate_ci_test_transfers,
    };

    println!("\n=== Executing Batch Native Token Transfers ===");
    println!("Number of transfers: {count}");

    // Generate transfer configs
    let configs = if query.ci_mode {
        println!("CI mode: Using test addresses");
        generate_ci_test_transfers(count, base_amount)
    } else {
        // For non-CI mode, generate sequential transfers to different addresses
        (0..count)
            .map(|i| {
                let to_address = format!("0x{:040x}", 0x1000000000000000u64 + i as u64);
                create_transfer_config(to_address, base_amount + (i as u64 * 1000))
            })
            .collect()
    };

    // Create workflow config
    let workflow_config =
        create_workflow_config(query.wait_attestation, query.auto_query, query.send_tx);

    // Execute transfers with workflow
    let results = execute_batch_transfers_with_query(
        &conn.eth_rpc_url,
        &conn.eth_private_key,
        &conn.cc3_rpc_url,
        &conn.cc3_evm_private_key,
        configs,
        workflow_config,
        query.chain_key,
    )
    .await?;

    println!("\nAll transfers completed successfully!");
    for (i, result) in results.iter().enumerate() {
        println!("Transfer {}:", i + 1);
        println!("  Transaction hash: 0x{:x}", result.transfer.tx_hash);
        println!("  Block number: {}", result.transfer.block_number);
        if let Some(attestation_block) = result.attestation_block {
            println!("  Attested at CC3 block: {attestation_block}");
        }
        if let Some(ref query_result) = result.query_result {
            println!("  Query executed: {}", query_result.success);
        }
    }

    Ok(())
}

pub async fn submit_native_query(params: NativeQueryParams) -> Result<(), Box<dyn Error>> {
    println!("\n=== Native Query Execution ===");

    // Step 1: Collect query parameters
    let prompt_args = PromptArgs {
        eth_rpc_url: params.eth_rpc_url,
        block_height: params.block_height,
        txn_hash: params.txn_hash,
        data_choice: params.data_choice,
    };

    let prompt_output = prompt_user(prompt_args).expect("Failed to prompt user");

    // Step 2: Fetch block data from source chain
    println!("\n=== Fetching Block Data ===");
    let query_eth_client = Client::new(&prompt_output.network.url(), None).await?;

    let block = query_eth_client
        .get_block(prompt_output.height, prompt_output.encoding)
        .await?;
    println!("Block fetched successfully");

    // Step 3: Check if block has transactions
    if block.items().is_empty() {
        return Err("Block has no transactions".into());
    }

    // Step 4: Find transaction by hash
    let tx_index = block
        .items()
        .iter()
        .position(|tx| tx.tx_hash().to_string() == prompt_output.tx_hash)
        .unwrap_or(0);

    println!("Using transaction at index {tx_index}");
    let tx_rx = &block.items()[tx_index];
    let full_tx_data = tx_rx.to_bytes();

    println!("\nDEBUG: Transaction verification details:");
    println!("  Transaction index: {tx_index}");
    println!("  Transaction data size: {} bytes", full_tx_data.len());
    println!(
        "  First 64 bytes: 0x{}",
        hex::encode(&full_tx_data[..64.min(full_tx_data.len())])
    );

    // Step 6: Display block information (using refactored module)
    merkle::display_block_info(&block);

    // Step 7: Generate Merkle proof (using refactored module)
    println!("\n=== Merkle Proof Generation ===");
    let merkle_proof = merkle::generate_merkle_proof(&block, tx_index)?;
    println!("Merkle root: {:?}", merkle_proof.root);
    println!("Siblings count: {}", merkle_proof.siblings.len());

    // Debug: print siblings
    for (i, sibling) in merkle_proof.siblings.iter().enumerate() {
        if sibling.hash == sp_core::H256::default() {
            println!("  Sibling[{i}]: PLACEHOLDER (is_left: {})", sibling.is_left);
        } else {
            println!(
                "  Sibling[{}]: 0x{} (is_left: {})",
                i,
                hex::encode(&sibling.hash.0[..8]),
                sibling.is_left
            );
        }
    }

    // Step 8: Generate continuity proof (using refactored module)
    println!("\n=== Continuity Proof Generation ===");
    println!("Configured chain key: {}", params.chain_key);
    let continuity_blocks = continuity::builder::fetch_continuity_proof(
        &params.cc3_rpc_url,
        &params.cc3_evm_private_key,
        &prompt_output.network.url(),
        params.chain_key,
        prompt_output.height,
    )
    .await?;
    println!("Continuity blocks: {}", continuity_blocks.len());

    // Debug: Check Merkle root match
    if let Some(query_block) = continuity_blocks
        .iter()
        .find(|b| b.block_number == prompt_output.height)
    {
        println!("\n=== Merkle Root Comparison ===");
        println!(
            "Query block {} root (from continuity): 0x{:?}",
            prompt_output.height, query_block.root
        );
        println!("Merkle proof root: 0x{:?}", merkle_proof.root);
        if query_block.root != merkle_proof.root {
            println!("⚠️  WARNING: Merkle root mismatch!");
            println!("   Continuity root: 0x{:?}", query_block.root);
            println!("   Proof root:      0x{:?}", merkle_proof.root);
            println!("   This will cause verification to fail.");
        } else {
            println!("✅ Merkle roots match!");
        }
    } else {
        println!(
            "⚠️  WARNING: Query block {} not found in continuity chain!",
            prompt_output.height
        );
    }

    // Step 9: Verify the query (using refactored module)
    println!("\n=== Query Verification ===");
    let verification_config = verification::VerificationConfig {
        cc3_rpc_url: params.cc3_rpc_url.clone(),
        cc3_evm_private_key: params.cc3_evm_private_key.clone(),
        eth_rpc_url: prompt_output.network.url(),
        chain_key: params.chain_key,
    };

    let result = verification::verify_query(
        &verification_config,
        params.chain_key,
        prompt_output.height,
        &full_tx_data,
        merkle_proof,
        continuity_blocks.clone(),
        params.send_tx,
    )
    .await?;
    verification::display_results(params.chain_key, prompt_output.height, &result);

    Ok(())
}

#[derive(Debug, Clone)]
pub struct PromptArgs {
    pub eth_rpc_url: Option<String>,
    pub block_height: Option<u64>,
    pub txn_hash: Option<String>,
    pub data_choice: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum Network {
    Sepolia(String),
    Ethereum(String),
    Local(String),
    Custom(u64, String),
}

impl Network {
    pub fn url(&self) -> String {
        match self {
            Network::Sepolia(api_key) => format!("wss://sepolia.infura.io/ws/v3/{api_key}"),
            Network::Ethereum(api_key) => format!("wss://mainnet.infura.io/ws/v3/{api_key}"),
            Network::Local(url) => url.clone(),
            Network::Custom(_, url) => url.clone(),
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            Network::Sepolia(_) => 11155111,
            Network::Ethereum(_) => 1,
            Network::Local(_) => 2,
            Network::Custom(id, _) => *id,
        }
    }
}

/// Handle interactive query mode
async fn handle_interactive_query(
    cc3_rpc_url: String,
    cc3_evm_private_key: String,
    chain_key: u64,
    send_tx: bool,
) -> Result<(), Box<dyn Error>> {
    // Get query configuration from user
    let prompt_args = PromptArgs {
        eth_rpc_url: None,
        block_height: None,
        txn_hash: None,
        data_choice: None,
    };
    let prompt_output = prompt::prompt(prompt_args)?;

    // Submit the query
    let params = NativeQueryParams {
        cc3_rpc_url,
        cc3_evm_private_key,
        eth_rpc_url: Some(prompt_output.network.url()),
        block_height: Some(prompt_output.height),
        txn_hash: Some(prompt_output.tx_hash),
        data_choice: Some(match prompt_output.selected_data {
            prompt::SelectedData::All => 0,
            prompt::SelectedData::RangeOfData => 1,
            prompt::SelectedData::Erc20TransferData => 2,
            prompt::SelectedData::NativeTokenTransferData => 3,
        }),
        chain_key,
        send_tx,
    };
    submit_native_query(params).await
}
