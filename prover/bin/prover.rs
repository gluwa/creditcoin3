use clap::Parser;
use std::error::Error;
use tokio::signal;
use tracing::{debug, info};

use prover::{config::Config, Server};

#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct Attestor {
    #[arg(long, default_value = "ws://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_key: String,

    #[arg(long, default_value = "http://localhost:8545")]
    eth_rpc_url: String,

    #[arg(long, required = true)]
    eth_private_key: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long, default_value_t = 100)]
    claim_buffer: u8,

    #[arg(
        long,
        default_value = "postgres://prover:prover@127.0.0.1:5432/attestations"
    )]
    postgres_uri: String,

    #[arg(long, required = false)]
    prover_be_socket_addr: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Attestor::parse();

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

    let config = Config {
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
        eth_rpc_url: args.eth_rpc_url,
        eth_private_key: args.eth_private_key,
        claim_buffer: args.claim_buffer,
        postgres_uri: args.postgres_uri,
        prover_be_socket_addr: args.prover_be_socket_addr,
    };

    let mut server = Server::new(config).await?;
    server.run().await?;

    // Wait for Ctrl+C signal
    signal::ctrl_c().await?;
    info!("Ctrl+C received, shutting down...");

    Ok(())
}
