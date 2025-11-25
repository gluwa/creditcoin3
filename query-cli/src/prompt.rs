use anyhow::Result;
use ccnext_abi_encoding::abi::EncodingVersion;
use std::io::{self, Write};

use crate::{Network, PromptArgs};

#[derive(Debug)]
pub(crate) struct PromptOutput {
    pub network: Network,
    pub height: u64,
    pub tx_hash: String,
    pub selected_data: SelectedData,
    pub encoding: EncodingVersion,
}

#[derive(Debug)]
pub(crate) enum SelectedData {
    All,
    RangeOfData,
    Erc20TransferData,
    NativeTokenTransferData,
}

pub(crate) fn prompt(args: PromptArgs) -> Result<PromptOutput> {
    // Prompt the user for the network
    let network = prompt_for_network(args.clone())?;

    // Prompt the user for block height and transaction hash
    let (tx_height, tx_hash) = prompt_for_height_and_hash(args.clone());

    // Prompt the user for all data, range of data, ERC20 preset data, or native transfer preset data
    let (selected_data, offsets_and_sizes) = prompt_for_data_selection(args);

    // For now we hardcode the only version we support
    let encoding = EncodingVersion::V1;

    // Display the collected information
    println!("\nCollected Information:");
    println!("Network: {network:?}");
    println!("Block Height: {tx_height}");
    println!("Transaction Hash: {}", tx_hash.trim());
    match selected_data {
        SelectedData::All => println!("Data: All data\n"),
        SelectedData::RangeOfData => {
            println!("Data: Range (offset & end: {offsets_and_sizes:?})\n")
        }
        SelectedData::Erc20TransferData => println!("Data: ERC 20 Transfer Data)\n"),
        SelectedData::NativeTokenTransferData => println!("Data: Native Token Transfer Data)\n"),
    };

    Ok(PromptOutput {
        height: tx_height,
        network,
        tx_hash: tx_hash.trim().to_string(),
        selected_data,
        encoding,
    })
}

fn prompt_for_network(args: PromptArgs) -> Result<Network> {
    if let Some(eth_rpc_url) = args.eth_rpc_url {
        return Ok(Network::Local(eth_rpc_url.trim().to_string()));
    }

    // Prompt the user for the network
    println!("Please select the network:");
    println!("1. Sepolia");
    println!("2. Ethereum");
    println!("3. Local");
    println!("4. Custom (provide ID and URL)");
    print!("Enter your choice (1, 2, 3 or 4): ");
    io::stdout().flush().unwrap(); // Flush stdout to ensure the prompt is shown

    let mut network_choice = String::new();
    io::stdin()
        .read_line(&mut network_choice)
        .expect("Failed to read input");

    match network_choice.trim() {
        "1" => {
            print!("Enter Sepolia Infura API key: ");
            io::stdout().flush().unwrap();

            let mut api_key_input = String::new();
            io::stdin()
                .read_line(&mut api_key_input)
                .expect("Failed to read input");

            Ok(Network::Sepolia(api_key_input.trim().to_string()))
        }
        "2" => {
            print!("Enter Ethereum Infura API key: ");
            io::stdout().flush().unwrap();

            let mut api_key_input = String::new();
            io::stdin()
                .read_line(&mut api_key_input)
                .expect("Failed to read input");

            Ok(Network::Ethereum(api_key_input.trim().to_string()))
        }
        "3" => {
            print!("Enter local network URL (EX: ws://localhost:8545): ");
            io::stdout().flush().unwrap();

            let mut url_input = String::new();
            io::stdin()
                .read_line(&mut url_input)
                .expect("Failed to read input");

            if url_input.trim().is_empty() {
                url_input = "ws://localhost:8545".to_string();
            }

            Ok(Network::Local(url_input.trim().to_string()))
        }
        "4" => {
            print!("Enter custom network ID: ");
            io::stdout().flush().unwrap();

            let mut id_input = String::new();
            io::stdin()
                .read_line(&mut id_input)
                .expect("Failed to read input");
            let id: u64 = id_input
                .trim()
                .parse()
                .expect("Please enter a valid number");

            print!("Enter custom network URL: ");
            io::stdout().flush().unwrap();

            let mut url_input = String::new();
            io::stdin()
                .read_line(&mut url_input)
                .expect("Failed to read input");

            Ok(Network::Custom(id, url_input.trim().to_string()))
        }
        _ => {
            // exit
            println!("Invalid choice. Exiting.");
            Err(anyhow::anyhow!("Invalid network choice"))
        }
    }
}

fn prompt_for_height_and_hash(args: PromptArgs) -> (u64, String) {
    let height = args.block_height.unwrap_or_else(|| {
        print!("Enter the block height (number): ");
        io::stdout().flush().unwrap();

        let mut height_input = String::new();
        io::stdin()
            .read_line(&mut height_input)
            .expect("Failed to read input");
        height_input
            .trim()
            .parse::<u64>()
            .expect("Please enter a valid number")
    });

    let tx_hash = args.txn_hash.unwrap_or_else(|| {
        print!("Enter the transaction hash: ");
        io::stdout().flush().unwrap();

        let mut tx_hash = String::new();
        io::stdin()
            .read_line(&mut tx_hash)
            .expect("Failed to read input");
        tx_hash
    });

    (height, tx_hash)
}

fn prompt_for_data_selection(args: PromptArgs) -> (SelectedData, Vec<(u64, u64)>) {
    let data_choice: u64 = args.data_choice.unwrap_or_else(|| {
        println!("Which data do you want represented in your proof results?");
        println!("1. All data");
        println!("2. Range of data");
        println!("3. ERC20 transfer data");
        println!("4. Native token transfer data");
        print!("Enter your choice (1, 2, 3, or 4): ");
        io::stdout().flush().unwrap();

        let mut data_choice = String::new();
        io::stdin()
            .read_line(&mut data_choice)
            .expect("Failed to read input");
        data_choice
            .trim()
            .parse::<u64>()
            .expect("Please enter a valid number")
    });

    let mut selected_data = SelectedData::RangeOfData;
    let mut offsets_and_sizes: Vec<(u64, u64)> = Vec::new();

    match data_choice {
        1 => {
            selected_data = SelectedData::All;
        }
        2 => loop {
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

            print!("Enter the size: ");
            io::stdout().flush().unwrap();

            let mut end_input = String::new();
            io::stdin()
                .read_line(&mut end_input)
                .expect("Failed to read input");
            let end: u64 = end_input
                .trim()
                .parse()
                .expect("Please enter a valid number");

            offsets_and_sizes.push((offset, end));

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
        3 => {
            selected_data = SelectedData::Erc20TransferData;
        }
        4 => {
            selected_data = SelectedData::NativeTokenTransferData;
        }
        _ => {
            println!("Invalid choice. Defaulting to all data.");
            selected_data = SelectedData::All;
        }
    };
    (selected_data, offsets_and_sizes)
}
