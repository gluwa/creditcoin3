use anyhow::Result;
use bip39::{Language, Mnemonic, MnemonicType};
use clap::Parser;
use rand::seq::SliceRandom;
use serde::Deserialize;
use std::str::FromStr;
use std::sync::Arc;
pub use subxt::utils::AccountId32;
use subxt_signer::{sr25519::Keypair, SecretUri};
use tempfile::{tempdir, TempDir};
use tokio::{
    fs::File,
    process::{Child, Command},
    signal::ctrl_c,
    sync::Mutex,
};
use tracing::{debug, info};

use cc_client::Client;

#[derive(Parser, Debug)]
#[command(name = "attestor_zombienet")]
pub struct AttestorZombienet {
    #[arg(
        long,
        default_value = "http://localhost:8545",
        help = "URL to an Ethereum node."
    )]
    eth_rpc_url: String,

    #[arg(
        long,
        default_value = "ws://localhost:9944",
        help = "A Creditcoin3 url to a node with rpc and websocket enabled"
    )]
    cc3_rpc_url: String,

    #[arg(
        long,
        required = true,
        help = "Mnemonic for a creditcoin3 account that will fund the attestors"
    )]
    cc3_key: String,

    #[arg(long, default_value = "config.yaml", help = "Path to the config file")]
    config_file: String,

    #[arg(
        long,
        required = false,
        help = "If set, override `chain_id` from config file"
    )]
    chain_id: Option<u64>,

    #[arg(
        long,
        short,
        help = "Cc3 node ports to connect to, will randomonly assign one of the ports to each attestor process to balance the load.",
        default_value = "9944,9945,9946,9947",
        value_delimiter = ','
    )]
    port_ranges: Vec<u64>,

    #[arg(short, long, help = "Turn on verbose logging")]
    verbose: bool,
}

#[derive(Debug, Deserialize)]
struct Process {
    run: bool,
    default_command: String,
    default_args: Option<Vec<String>>,
    num_attestors: u64,
    single_node: bool,
    chain_id: u64,
}

#[derive(Debug, Clone)]
struct AttestorKey {
    keypair: Keypair,
    secret: String,
}

const BATCH_SIZE: usize = 50;
const TIMEOUT_SECONDS: u64 = 6;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = AttestorZombienet::parse();

    // enable tracing debug logs if verbose flag is set
    let env_filter = if args.verbose {
        debug!("debug mode enabled!");
        "debug"
    } else {
        "attestor_zombienet=info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(true)
        .with_env_filter(env_filter)
        .try_init();

    // Load and parse the config file
    let mut config: Process = {
        let config_str = std::fs::read_to_string(args.config_file)?;
        serde_yaml::from_str(&config_str)?
    };
    config.chain_id = args.chain_id.unwrap_or(config.chain_id);

    let mut keys = vec![];
    for _ in 0..config.num_attestors {
        let mnemonic = Mnemonic::new(MnemonicType::Words12, Language::English);
        let phrase = mnemonic.phrase();
        let secret_uri = SecretUri::from_str(phrase).expect("Failed to create secret uri");
        let keypair = Keypair::from_uri(&secret_uri).expect("Failed to create keypair");

        keys.push(AttestorKey {
            keypair,
            secret: phrase.to_string(),
        });
    }

    info!("Will start funding {} attestor keys", keys.len());

    let cc_client = Client::new(args.cc3_rpc_url, &args.cc3_key)
        .await
        .expect("Failed to create client");

    let mut nonce = cc_client
        .get_account_nonce()
        .await
        .expect("faild to get nonce");

    // fund keys in batches
    let key_chunks: Vec<Vec<AttestorKey>> = keys
        .chunks(BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect();

    for chunk in key_chunks {
        let chunks = chunk.clone();
        let futures = chunks.into_iter().map(|key| {
            let cc_client = cc_client.clone();
            let attestor = AccountId32(key.keypair.public_key().0);

            info!("Transferring 10 dev CTC to {}", attestor);
            let handle = tokio::spawn(async move {
                cc_client
                    .transfer(
                        key.keypair.public_key().into(),
                        10_000_000_000_000_000_000,
                        Some(nonce),
                    )
                    .await
                    .expect("Failed to transfer")
            });
            nonce += 1;

            handle
        });

        // wait for all transfers in the current batch to complete
        futures::future::join_all(futures).await;

        // set a timeout after each batch
        tokio::time::sleep(tokio::time::Duration::from_secs(TIMEOUT_SECONDS)).await;
    }

    info!("All attestor keys funded!\n");
    tokio::time::sleep(tokio::time::Duration::from_secs(15)).await;

    let mut nonce = cc_client
        .get_account_nonce()
        .await
        .expect("faild to get nonce");

    // Now create attestors for each funded key
    // fund keys in batches
    let key_chunks: Vec<Vec<AttestorKey>> = keys
        .chunks(BATCH_SIZE)
        .map(|chunk| chunk.to_vec())
        .collect();

    for chunk in key_chunks {
        let chunks = chunk.clone();
        let futures = chunks.into_iter().map(|key| {
            let cc_client = cc_client.clone();
            let attestor = AccountId32(key.keypair.public_key().0);

            info!(
                "Registering attestor with address({}) for chain: {}",
                attestor, config.chain_id
            );
            let handle = tokio::spawn(async move {
                cc_client
                    .register_attestor(config.chain_id, attestor, Some(nonce))
                    .await
                    .expect("Failed to register attestor")
            });
            nonce += 1;

            handle
        });

        // wait for all transfers in the current batch to complete
        futures::future::join_all(futures).await;

        // set a timeout after each batch
        tokio::time::sleep(tokio::time::Duration::from_secs(TIMEOUT_SECONDS)).await;
    }

    info!("All attestors registered!\n");
    tokio::time::sleep(tokio::time::Duration::from_secs(TIMEOUT_SECONDS)).await;

    if config.run {
        // Create an Arc and Mutex to manage child processes
        let children = Arc::new(Mutex::new(Vec::new()));

        let temp_dir = tempdir()?;
        // Spawn child processes
        let mut rng = rand::thread_rng();

        for attestor_key in keys {
            let command = config.default_command.clone();
            let mut attestor_args = config.default_args.clone().unwrap_or_default();

            attestor_args.push(format!("--cc3-key={}", attestor_key.secret));
            attestor_args.push(format!("--eth-rpc-url={}", args.eth_rpc_url));

            // get random number out of args.port_ranges
            // if single_node is true, use 9944
            let port = match config.single_node {
                true => 9944,
                false => *args.port_ranges.choose(&mut rng).unwrap(),
            };

            attestor_args.push(format!("--cc3-rpc-url=ws://localhost:{}", port));
            if args.verbose {
                attestor_args.push("--verbose".to_string());
            }

            let attestor = AccountId32(attestor_key.keypair.public_key().0);

            let child = spawn_child(&command, &attestor_args, &temp_dir, attestor).await?;
            children.lock().await.push(child);
        }

        // Handle Ctrl+C signal
        let children_clone = Arc::clone(&children);
        tokio::spawn(async move {
            ctrl_c().await.expect("failed to listen for event");
            info!("Ctrl+C received, terminating child processes...");
            let mut children = children_clone.lock().await;
            for child in children.iter_mut() {
                child.kill().await.expect("failed to kill process");
            }
            info!("All child processes terminated.");
            std::process::exit(0);
        });

        // // Prevent main from exiting
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    } else {
        info!("Not running");
        Ok(())
    }
}

async fn spawn_child(
    command: &str,
    args: &[String],
    tempdir: &TempDir,
    attestor: AccountId32,
) -> Result<Child> {
    // Create a temporary file for the process output
    let file_path = tempdir.path().join(format!("child-{}.txt", attestor));
    let tmp_file = File::create(file_path.clone()).await?;

    // Redirect the stdout and stderr of the child process to the temporary file
    let std_out_ouput = tmp_file.try_clone().await?.into_std().await;
    let std_err_ouput = tmp_file.try_clone().await?.into_std().await;

    let child = Command::new(command)
        .args(args)
        .stdout(std_out_ouput)
        .stderr(std_err_ouput)
        .spawn()?;

    // Log start up
    info!("Attestor {attestor} started!");
    info!("Log Cmd: tail -f {}\n", file_path.display());

    Ok(child)
}
