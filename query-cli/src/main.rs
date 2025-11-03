use anyhow::Result;
use ccnext_abi_encoding::abi::EncodingVersion;
use clap::{Parser, Subcommand};
use std::error::Error;
use utils::block_item_traits::BlockItem;

mod config;
mod continuity;
mod merkle;
mod native_query;
mod prompt;
mod query_builder;
mod verification;

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
enum Commands {
    /// Submit query through the prover contract (old way with ZK proofs)
    Prover {
        #[arg(long, required = true)]
        prover_contract_address: String,

        #[arg(long, default_value = "false")]
        default: bool,

        #[arg(long)]
        eth_rpc_url: Option<String>,

        #[arg(long)]
        block_height: Option<u64>,

        #[arg(long)]
        txn_hash: Option<String>,

        #[arg(long)]
        data_choice: Option<u64>,
    },
    /// Verify query through the native precompile (new way - direct verification)
    Native {
        #[arg(long)]
        eth_rpc_url: Option<String>,

        #[arg(long)]
        block_height: Option<u64>,

        #[arg(long)]
        txn_hash: Option<String>,

        #[arg(long)]
        data_choice: Option<u64>,
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
        Commands::Prover {
            prover_contract_address,
            default,
            eth_rpc_url,
            block_height,
            txn_hash,
            data_choice,
        } => {
            if default {
                submit_default_query(
                    args.cc3_rpc_url,
                    args.cc3_evm_private_key,
                    prover_contract_address,
                )
                .await?;
                return Ok(());
            }

            submit_prover_query(
                args.cc3_rpc_url,
                args.cc3_evm_private_key,
                prover_contract_address,
                eth_rpc_url,
                block_height,
                txn_hash,
                data_choice,
            )
            .await?;
        }
        Commands::Native {
            eth_rpc_url,
            block_height,
            txn_hash,
            data_choice,
        } => {
            // If all parameters are provided via CLI, use direct execution
            // Otherwise, use interactive mode
            if eth_rpc_url.is_some() && block_height.is_some() && txn_hash.is_some() {
                submit_native_query(
                    args.cc3_rpc_url,
                    args.cc3_evm_private_key,
                    eth_rpc_url,
                    block_height,
                    txn_hash,
                    data_choice,
                )
                .await?;
            } else {
                // Use the refactored interactive handler
                native_query::submission::handle_interactive(
                    args.cc3_rpc_url,
                    args.cc3_evm_private_key,
                )
                .await?;
            }
        }
    }

    Ok(())
}

async fn submit_prover_query(
    cc3_rpc_url: String,
    cc3_evm_private_key: String,
    prover_contract_address: String,
    eth_rpc_url: Option<String>,
    block_height: Option<u64>,
    txn_hash: Option<String>,
    data_choice: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    use crate::prompt::{prompt as prompt_user, SelectedData};
    use crate::query_builder::{get_erc20_transfer_segments, get_native_token_transfer_segments};
    use attestor_primitives::{LayoutSegment, Query};
    use eth::Client;

    let prompt_args = PromptArgs {
        eth_rpc_url,
        block_height,
        txn_hash,
        data_choice,
    };

    let prompt_output = prompt_user(prompt_args).expect("Failed to prompt user");

    let query_eth_client = Client::new(&prompt_output.network.url(), None).await?;

    let block = query_eth_client
        .get_block(prompt_output.height, prompt_output.encoding)
        .await?;
    // Get tx index
    let tx_index = block
        .items()
        .iter()
        .position(|tx_rx| tx_rx.tx_hash().to_string() == prompt_output.tx_hash)
        .expect("Transaction not found in block");

    let tx_rx = &block.items()[tx_index];

    let data = tx_rx.to_bytes();

    let layout_segments = match prompt_output.selected_data {
        SelectedData::All => {
            vec![LayoutSegment {
                offset: 0,
                size: data.len() as u64,
            }]
        }
        SelectedData::RangeOfData => prompt_output
            .offsets_and_sizes
            .iter()
            .map(|(offset, size)| LayoutSegment {
                offset: *offset,
                size: *size,
            })
            .collect(),
        SelectedData::Erc20TransferData => {
            get_erc20_transfer_segments(
                prompt_output.network.clone(),
                tx_rx.tx().clone(),
                tx_rx.rx().clone(),
                prompt_output.encoding,
            )
            .await?
        }
        SelectedData::NativeTokenTransferData => {
            get_native_token_transfer_segments(
                prompt_output.network.clone(),
                tx_rx.tx().clone(),
                tx_rx.rx().clone(),
                prompt_output.encoding,
            )
            .await?
        }
    };

    let query = Query {
        height: prompt_output.height,
        chain_id: prompt_output.network.id(),
        index: tx_index as u64,
        layout_segments,
    };

    let query_id = query.id();
    println!("Query ID: {query_id:?}");
    println!("Going to submit following Query: {query:?}\n");

    // Initialize the Ethereum client for ccnext and the contract
    let eth_client = Client::new(&cc3_rpc_url, Some(&cc3_evm_private_key)).await?;
    let contract = eth::evm::prover::new(prover_contract_address)?;

    println!("Checking for existing result...");
    if let Some(result_segments) = contract
        .get_query_result(&eth_client, query.clone())
        .await?
    {
        println!("\nResult segments already available: {result_segments:?}");
        return Ok(());
    }

    println!("\nNo existing result found, proceeding with query submission...");

    println!("\nComputing query cost...");
    let computed_cost = contract
        .compute_query_cost(&eth_client, query.clone())
        .await?;
    println!("Computed cost: {computed_cost}\n");

    println!("Submitting query...");
    let tx_hash = contract
        .submit_query(&eth_client, query, computed_cost)
        .await?;
    println!("Query submitted! Tx hash: {tx_hash}\n");

    println!("Waiting for result segments...");
    let result_segments = contract
        .subscribe_proof_verification(&eth_client, query_id.0.into())
        .await?;

    println!("\nResult segments received: {result_segments:?}");

    Ok(())
}

async fn submit_native_query(
    cc3_rpc_url: String,
    cc3_evm_private_key: String,
    eth_rpc_url: Option<String>,
    block_height: Option<u64>,
    txn_hash: Option<String>,
    data_choice: Option<u64>,
) -> Result<(), Box<dyn Error>> {
    use crate::prompt::{prompt as prompt_user, SelectedData};
    use crate::query_builder::{get_erc20_transfer_segments, get_native_token_transfer_segments};
    use attestor_primitives::{LayoutSegment, Query};
    use eth::Client;

    println!("\n=== Native Query Execution (Refactored) ===");

    // Step 1: Collect query parameters
    let prompt_args = PromptArgs {
        eth_rpc_url,
        block_height,
        txn_hash,
        data_choice,
    };

    let prompt_output = prompt_user(prompt_args).expect("Failed to prompt user");

    // Step 2: Fetch block data from source chain
    println!("\n=== Fetching Block Data ===");
    let query_eth_client = Client::new(&prompt_output.network.url(), None).await?;

    let block = query_eth_client
        .get_block(prompt_output.height, prompt_output.encoding)
        .await?;
    println!("Block fetched successfully");

    // Step 3: Find transaction index
    let tx_index = block
        .items()
        .iter()
        .position(|tx_rx| tx_rx.tx_hash().to_string() == prompt_output.tx_hash)
        .expect("Transaction not found in block");
    println!("Found transaction at index {tx_index}");

    let tx_rx = &block.items()[tx_index];
    let full_tx_data = tx_rx.to_bytes();

    println!("\nDEBUG: Transaction verification details:");
    println!("  Transaction index: {tx_index}");
    println!("  Transaction data size: {} bytes", full_tx_data.len());
    println!(
        "  First 64 bytes: 0x{}",
        hex::encode(&full_tx_data[..64.min(full_tx_data.len())])
    );

    // Step 4: Build layout segments based on data selection
    let identifier_size = 0u64;

    let layout_segments = match prompt_output.selected_data {
        SelectedData::All => {
            vec![LayoutSegment {
                offset: identifier_size,
                size: full_tx_data.len() as u64,
            }]
        }
        SelectedData::RangeOfData => prompt_output
            .offsets_and_sizes
            .iter()
            .map(|(offset, size)| LayoutSegment {
                offset: offset + identifier_size,
                size: *size,
            })
            .collect(),
        SelectedData::Erc20TransferData => {
            let mut segments = get_erc20_transfer_segments(
                prompt_output.network.clone(),
                tx_rx.tx().clone(),
                tx_rx.rx().clone(),
                prompt_output.encoding,
            )
            .await?;
            for segment in &mut segments {
                segment.offset += identifier_size;
            }
            segments
        }
        SelectedData::NativeTokenTransferData => {
            let mut segments = get_native_token_transfer_segments(
                prompt_output.network.clone(),
                tx_rx.tx().clone(),
                tx_rx.rx().clone(),
                prompt_output.encoding,
            )
            .await?;
            for segment in &mut segments {
                segment.offset += identifier_size;
            }
            segments
        }
    };

    // Step 5: Create the query
    let query = Query {
        height: prompt_output.height,
        chain_id: prompt_output.network.id(),
        index: tx_index as u64,
        layout_segments,
    };

    println!("\nQuery ID: {:?}", query.id());
    println!("Query details: {query:?}");

    // Step 6: Display block information (using refactored module)
    merkle::display_block_info(&block);

    // Step 7: Generate Merkle proof (using refactored module)
    println!("\n=== Merkle Proof Generation ===");
    let merkle_proof = merkle::generate_merkle_proof(&block, tx_index)?;
    println!("Merkle root: {:?}", merkle_proof.root);
    println!("Siblings count: {}", merkle_proof.siblings.len());

    // Debug: print siblings
    for (i, sibling) in merkle_proof.siblings.iter().enumerate() {
        if *sibling == sp_core::H256::default() {
            println!("  Sibling[{}]: PLACEHOLDER", i);
        } else {
            println!("  Sibling[{}]: 0x{}", i, hex::encode(&sibling.0[..8]));
        }
    }

    // Step 8: Generate continuity proof (using refactored module)
    println!("\n=== Continuity Proof Generation ===");
    let continuity_blocks = continuity::fetch_continuity_proof(
        &cc3_rpc_url,
        &query,
        &block,
        &prompt_output.network.url(),
    )
    .await?;
    println!("Continuity blocks: {}", continuity_blocks.len());

    // Step 9: Verify the query (using refactored module)
    println!("\n=== Query Verification ===");
    let verification_config = verification::VerificationConfig {
        cc3_rpc_url: cc3_rpc_url.clone(),
        cc3_evm_private_key: cc3_evm_private_key.clone(),
    };

    let result = verification::verify_query(
        &verification_config,
        &query,
        &full_tx_data,
        merkle_proof,
        continuity_blocks,
    )
    .await?;

    // Step 10: Display results (using refactored module)
    verification::display_results(&query, &result);

    Ok(())
}

/// Submit a default query for testing purposes
pub async fn submit_default_query(
    _cc3_rpc_url: String,
    cc3_evm_private_key: String,
    prover_contract_address: String,
) -> Result<()> {
    use attestor_primitives::{LayoutSegment, Query};

    let query = Query {
        chain_id: 2, // Local network
        height: 6493200,
        index: 75,
        layout_segments: vec![LayoutSegment {
            offset: 0,
            size: 99326,
        }],
    };

    let query_id = query.id();

    println!(
        "Going to submit following Query: {:?}, id({:?})\n",
        query, query_id
    );

    let eth_rpc_url = "ws://localhost:8545".to_string(); // Local Ethereum node URL

    // Initialize the Ethereum client for ccnext and the contract
    use eth::Client;
    let eth_client = Client::new(&eth_rpc_url, Some(&cc3_evm_private_key)).await?;
    let contract = eth::evm::prover::new(prover_contract_address)?;

    println!("Computing query cost...");
    let computed_cost = contract
        .compute_query_cost(&eth_client, query.clone())
        .await?;
    println!("Computed cost: {computed_cost}\n");

    println!("Submitting query...");
    let tx_hash = contract
        .submit_query(&eth_client, query, computed_cost)
        .await?;
    println!("Query submitted! Tx hash: {tx_hash}'n");

    println!("Waiting for proof...");
    let proof = contract
        .subscribe_proof_verification(&eth_client, query_id.0.into())
        .await?;

    println!("\nProof received: proof len: {}", proof.len());

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
