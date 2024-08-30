use anyhow::Result;
use clap::Parser;
use std::error::Error;
use std::io::{self, Write};
use tokio::sync::mpsc;
use tracing::debug;

use eth::{evm, Client};
use prover_primitives::{LayoutSegment, Query};
use utils::block_item_traits::BlockItem;

#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct QueryCli {
    #[arg(long)]
    eth_rpc_url: Option<String>,

    #[arg(long, required = true, default_value = "http://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    eth_private_key: String,

    #[arg(long)]
    infura_api_key: Option<String>,

    #[arg(long, required = true)]
    contract_address: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long, default_value = "false")]
    default: bool,
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

    let prompt: PromptOutput = prompt().expect("Failed to prompt user");

    let infura_api_key = args.infura_api_key.ok_or_else(|| {
        anyhow::anyhow!("Please provide an Infura API key (--infura-api-key 'somekey')")
    })?;

    let query_eth_client = Client::new(
        &prompt.network.url(infura_api_key, args.eth_rpc_url),
        &String::new(),
    )
    .await?;

    let block = query_eth_client.get_block(prompt.height).await?;
    // Get tx index
    let tx_index = block
        .items()
        .iter()
        .position(|tx_rx| tx_rx.tx_hash().to_string() == prompt.tx_hash)
        .expect("Transaction not found in block");

    let tx_rx = &block.items()[tx_index];
    let data = tx_rx.payload_bytes();

    let layout_segments = if !prompt.offset_end_ranges.is_empty() {
        prompt
            .offset_end_ranges
            .iter()
            .map(|(offset, size)| LayoutSegment {
                offset: *offset,
                size: *size,
            })
            .collect()
    } else {
        vec![LayoutSegment {
            offset: 0,
            size: data.len() as u64,
        }]
    };

    let query = Query {
        height: prompt.height,
        chain_id: prompt.network.id(),
        index: tx_index as u64,
        layout_segments,
    };

    let query_id = query.id();
    println!("Query ID: {:?}", query_id);
    println!("Going to submit following Query: {:?}\n", query);

    // Initialize the Ethereum client for ccnext and the contract
    let eth_client = Client::new(&args.cc3_rpc_url, &args.eth_private_key).await?;
    let contract = evm::prover::new(args.contract_address)?;

    println!("Computing query cost...");
    let computed_cost = contract
        .compute_query_cost(&eth_client, query.clone())
        .await?;
    println!("Computed cost: {}\n", computed_cost);

    println!("Submitting query...");
    let tx_hash = contract
        .submit_query(&eth_client, query, computed_cost)
        .await?;
    println!("Query submitted! Tx hash: {}", tx_hash);

    let (proof_channel_sender, mut proof_chan_rec) = mpsc::channel(1);

    tokio::spawn(async move {
        let proof = contract
            .subscribe_proof_verification(&eth_client, query_id.0.into(), proof_channel_sender)
            .await;
        if let Err(e) = proof {
            println!("Error: {:?}", e);
        }
    });

    // await result
    let proof = proof_chan_rec.recv().await.unwrap();
    println!("\nProof received: proof len: {}", proof.len());

    Ok(())
}

pub async fn submit_default_query(args: QueryCli) -> Result<()> {
    let query = Query {
        chain_id: Network::Sepolia.id(),
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

    let eth_rpc_url = args.eth_rpc_url.unwrap_or_else(|| {
        debug!("Using default eth rpc url");
        "http://localhost:8545".to_string()
    });

    // Initialize the Ethereum client for ccnext and the contract
    let eth_client = Client::new(&eth_rpc_url, &args.eth_private_key).await?;
    let contract = evm::prover::new(args.contract_address)?;

    println!("Computing query cost...");
    let computed_cost = contract
        .compute_query_cost(&eth_client, query.clone())
        .await?;
    println!("Computed cost: {}\n", computed_cost);

    println!("Submitting query...");
    let tx_hash = contract
        .submit_query(&eth_client, query, computed_cost)
        .await?;
    println!("Query submitted! Tx hash: {}", tx_hash);

    let (proof_channel_sender, mut proof_chan_rec) = mpsc::channel(1);

    tokio::spawn(async move {
        let proof = contract
            .subscribe_proof_verification(&eth_client, query_id.0.into(), proof_channel_sender)
            .await;
        if let Err(e) = proof {
            println!("Error: {:?}", e);
        }
    });

    // await result
    let proof = proof_chan_rec.recv().await.unwrap();
    println!("\nProof received: proof len: {}", proof.len());

    Ok(())
}

#[derive(Debug)]
enum Network {
    Sepolia,
    Ethereum,
    Local,
}

impl Network {
    pub fn url(&self, api_key: String, custom_url: Option<String>) -> String {
        match self {
            Network::Sepolia => format!("https://sepolia.infura.io/v3/{api_key}"),
            Network::Ethereum => format!("https://mainnet.infura.io/v3/{api_key}"),
            Network::Local => custom_url.expect("Custom URL required for local network"),
        }
    }

    pub fn id(&self) -> u64 {
        match self {
            Network::Ethereum => 1,
            Network::Local => 2,
            Network::Sepolia => 3,
        }
    }
}

#[derive(Debug)]
struct PromptOutput {
    pub network: Network,
    pub height: u64,
    pub tx_hash: String,
    pub offset_end_ranges: Vec<(u64, u64)>,
}

fn prompt() -> Result<PromptOutput> {
    // Prompt the user for the network
    println!("Please select the network:");
    println!("1. Sepolia");
    println!("2. Ethereum");
    println!("3. Local");
    print!("Enter your choice (1, 2 or 3): ");
    io::stdout().flush().unwrap(); // Flush stdout to ensure the prompt is shown

    let mut network_choice = String::new();
    io::stdin()
        .read_line(&mut network_choice)
        .expect("Failed to read input");

    let network = match network_choice.trim() {
        "1" => Network::Sepolia,
        "2" => Network::Ethereum,
        "3" => Network::Local,
        _ => {
            println!("Invalid choice. Defaulting to Sepolia.");
            Network::Sepolia
        }
    };

    // Prompt the user for the height
    print!("Enter the block height (number): ");
    io::stdout().flush().unwrap();

    let mut height_input = String::new();
    io::stdin()
        .read_line(&mut height_input)
        .expect("Failed to read input");
    let height: u64 = height_input
        .trim()
        .parse()
        .expect("Please enter a valid number");

    // Prompt the user for the transaction hash
    print!("Enter the transaction hash: ");
    io::stdout().flush().unwrap();

    let mut tx_hash = String::new();
    io::stdin()
        .read_line(&mut tx_hash)
        .expect("Failed to read input");

    // Prompt the user for all data or a range of data
    println!("Do you want all data or a range of data?");
    println!("1. All data");
    println!("2. Range of data");
    print!("Enter your choice (1 or 2): ");
    io::stdout().flush().unwrap();

    let mut data_choice = String::new();
    io::stdin()
        .read_line(&mut data_choice)
        .expect("Failed to read input");

    let mut all_data = false;
    let mut offset_end_ranges: Vec<(u64, u64)> = Vec::new();

    match data_choice.trim() {
        "1" => {
            all_data = true;
        }
        "2" => loop {
            print!("Enter the offset: ");
            io::stdout().flush().unwrap();

            let mut offset_input = String::new();
            io::stdin()
                .read_line(&mut offset_input)
                .expect("Failed to read input");
            let offset: u64 = offset_input
                .trim()
                .parse()
                .expect("Please enter a valid number");

            print!("Enter the end: ");
            io::stdout().flush().unwrap();

            let mut end_input = String::new();
            io::stdin()
                .read_line(&mut end_input)
                .expect("Failed to read input");
            let end: u64 = end_input
                .trim()
                .parse()
                .expect("Please enter a valid number");

            if offset < end {
                offset_end_ranges.push((offset, end));
            } else {
                println!("Invalid range: offset should be less than end. Please try again.");
                continue;
            }

            print!("Do you want to add another range? (y/n): ");
            io::stdout().flush().unwrap();

            let mut continue_choice = String::new();
            io::stdin()
                .read_line(&mut continue_choice)
                .expect("Failed to read input");

            if continue_choice.trim().eq_ignore_ascii_case("n") {
                break;
            }
        },
        _ => {
            println!("Invalid choice. Defaulting to all data.");
            all_data = true;
        }
    }

    // Display the collected information
    println!("\nCollected Information:");
    println!("Network: {:?}", network);
    println!("Block Height: {}", height);
    println!("Transaction Hash: {}", tx_hash.trim());
    if all_data {
        println!("Data: All data\n");
    } else {
        println!("Data: Range (offset & end: {:?})\n", offset_end_ranges);
    }

    Ok(PromptOutput {
        height,
        network,
        tx_hash: tx_hash.trim().to_string(),
        offset_end_ranges,
    })
}
