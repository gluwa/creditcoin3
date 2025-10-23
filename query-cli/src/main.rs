use anyhow::Result;
use ccnext_abi_encoding::abi::EncodingVersion;
use clap::Parser;
use prompt::{prompt, PromptOutput, SelectedData};
use query_builder::{get_erc20_transfer_segments, get_native_token_transfer_segments};
use std::error::Error;
use tracing::debug;

use eth::{evm, Client};
use pallet_prover_primitives::{LayoutSegment, Query};
use utils::block_item_traits::BlockItem;

mod prompt;
mod query_builder;
#[cfg(test)]
mod tests;

#[derive(Parser, Debug, Clone)]
#[command(name = "attestor")]
pub struct QueryCli {
    #[arg(long, required = true, default_value = "http://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_evm_private_key: String,

    #[arg(long, required = true)]
    prover_contract_address: String,

    #[arg(short, long)]
    verbose: bool,

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = QueryCli::parse();

    // enable tracing debug logs if verbose flag is set
    let env_filter = if args.verbose {
        debug!("debug mode enabled!");
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

    if args.default {
        submit_default_query(args).await?;
        return Ok(());
    }

    let prompt: PromptOutput = prompt(args.clone()).expect("Failed to prompt user");

    let query_eth_client = Client::new(&prompt.network.url(), None).await?;

    let block = query_eth_client
        .get_block(prompt.height, prompt.encoding)
        .await?;
    // Get tx index
    let tx_index = block
        .items()
        .iter()
        .position(|tx_rx| tx_rx.tx_hash().to_string() == prompt.tx_hash)
        .expect("Transaction not found in block");

    let tx_rx = &block.items()[tx_index];

    let data = tx_rx.payload_bytes();

    let layout_segments = match prompt.selected_data {
        SelectedData::All => {
            vec![LayoutSegment {
                offset: 0,
                size: data.len() as u64,
            }]
        }
        SelectedData::RangeOfData => prompt
            .offsets_and_sizes
            .iter()
            .map(|(offset, size)| LayoutSegment {
                offset: *offset,
                size: *size,
            })
            .collect(),
        SelectedData::Erc20TransferData => {
            get_erc20_transfer_segments(
                prompt.network.clone(),
                tx_rx.tx().clone(),
                tx_rx.rx().clone(),
                prompt.encoding,
            )
            .await?
        }
        SelectedData::NativeTokenTransferData => {
            get_native_token_transfer_segments(
                prompt.network.clone(),
                tx_rx.tx().clone(),
                tx_rx.rx().clone(),
                prompt.encoding,
            )
            .await?
        }
    };

    let query = Query {
        height: prompt.height,
        chain_id: prompt.network.id(),
        index: tx_index as u64,
        layout_segments,
    };

    let query_id = query.id();
    println!("Query ID: {query_id:?}");
    println!("Going to submit following Query: {query:?}\n");

    // Initialize the Ethereum client for ccnext and the contract
    let eth_client = Client::new(&args.cc3_rpc_url, Some(&args.cc3_evm_private_key)).await?;
    let contract = evm::prover::new(args.prover_contract_address)?;

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

pub async fn submit_default_query(args: QueryCli) -> Result<()> {
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

    println!("Going to submit following Query: {query:?}, id({query_id:?})\n");

    let eth_rpc_url = "ws://localhost:8545".to_string(); // Local Ethereum node URL

    // Initialize the Ethereum client for ccnext and the contract
    let eth_client = Client::new(&eth_rpc_url, Some(&args.cc3_evm_private_key)).await?;
    let contract = evm::prover::new(args.prover_contract_address)?;

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
enum Network {
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
            Network::Sepolia(_) => 3,
            Network::Ethereum(_) => 1,
            Network::Local(_) => 2,
            Network::Custom(id, _) => *id,
        }
    }
}
