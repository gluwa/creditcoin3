use clap::Parser;
use std::error::Error;
use tracing::debug;
use tracing_subscriber::EnvFilter;

use attestor::{Config, Server};

#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct Attestor {
    #[arg(
        long,
        default_value = "ws://localhost:8545",
        help = "A websocket url to an ethereum node, must have websocket enabled and all the necessary rpc methods."
    )]
    eth_rpc_url: String,

    #[arg(
        long,
        help = "Start block for the source chain, if not provided it will start from the latest block. If provided, it will start from the provided block and subscribe to latest heads when it reached the latest head."
    )]
    eth_start_block: Option<u64>,

    #[arg(
        long,
        default_value = "http://localhost:9944",
        help = "A Creditcoin3 url to a node with rpc and websocket enabled"
    )]
    cc3_rpc_url: String,

    #[arg(long, required = true, help = "Mnemonic for a creditcoin3 account")]
    cc3_key: String,

    #[arg(short, long, help = "Turn on verbose logging")]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Attestor::parse();

    // enable tracing debug logs if verbose flag is set
    if args.verbose {
        std::env::set_var("RUST_LOG", "debug");
        debug!("debug mode enabled!");
    }

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();

    let config = Config {
        eth_rpc_url: args.eth_rpc_url,
        eth_start_block: args.eth_start_block,
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
        //bls_key: args.bls_key[..].try_into().unwrap(),
    };

    let server = Server::new(config);

    server.run().await?;

    Ok(())
}
