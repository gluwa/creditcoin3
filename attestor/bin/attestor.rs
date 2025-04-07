use clap::Parser;
use std::error::Error;
use tracing::debug;

use attestor::{Config, Server};

#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct Attestor {
    #[arg(
        long,
        default_value = "http://localhost:8545",
        help = "A websocket url to an ethereum node, must have websocket enabled and all the necessary rpc methods."
    )]
    eth_rpc_url: String,

    #[arg(
        long,
        default_value = "ws://localhost:9944",
        help = "A Creditcoin3 url to a node with rpc and websocket enabled"
    )]
    cc3_rpc_url: String,

    #[arg(long, required = true, help = "Mnemonic for a creditcoin3 account")]
    cc3_key: String,

    #[arg(short, long, help = "Turn on verbose logging")]
    verbose: bool,

    #[arg(long, default_value = "0", help = "Start block for the source chain")]
    start_block: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Attestor::parse();

    // enable tracing debug logs if verbose flag is set
    let env_filter = if args.verbose {
        debug!("debug mode enabled!");
        "debug"
    } else {
        "attestor=info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(true)
        .with_env_filter(env_filter)
        .try_init();

    let config = Config {
        eth_rpc_url: args.eth_rpc_url,
        eth_start_block: args.start_block,
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
    };

    let mut server = Server::new(config);

    server.run().await?;

    Ok(())
}
