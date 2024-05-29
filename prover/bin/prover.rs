use clap::Parser;
use std::error::Error;
use tokio::signal;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

use prover::{Config, Server};

#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct Attestor {
    #[arg(long, default_value = "ws://localhost:8545")]
    eth_rpc_url: String,

    #[arg(long, default_value = "http://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_key: String,

    #[arg(long, required = true)]
    nickname: String,

    #[arg(short, long)]
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
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
        nickname: args.nickname,
    };

    let mut server = Server::new(config);
    server.run().await?;

    // Wait for Ctrl+C signal
    signal::ctrl_c().await?;
    info!("Ctrl+C received, shutting down...");

    Ok(())
}
