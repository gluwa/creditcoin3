use attestor_primitives::ChainKey;
use clap::Parser;
use dotenv::dotenv;
use prover::{config::Config, Server};
use std::error::Error;
use tokio::signal;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(name = "prover")]
pub struct Prover {
    #[arg(long, default_value = "ws://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_key: String,

    #[arg(long, default_value = "http://localhost:8545")]
    eth_rpc_url: String,

    #[arg(long, required = true, env)]
    cc3_evm_private_key: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long, default_value_t = 2)]
    chain_key: ChainKey,

    #[arg(long, default_value_t = 100)]
    claim_buffer: u8,

    #[arg(long, required = false, env)]
    postgres_uri: String,

    #[arg(long, required = false)]
    prover_be_socket_addr: Option<String>,

    #[arg(long, required = false, env)]
    be_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize environment file
    dotenv().ok();

    // Parse args
    let args = Prover::parse();

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

    if (args.prover_be_socket_addr.is_some() && args.be_api_key.is_none())
        || (args.prover_be_socket_addr.is_none() && args.be_api_key.is_some())
    {
        panic!("Bad arguments! prover-be-socket-addr and be-api-key must be specified together.");
    }

    let config = Config {
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
        eth_rpc_url: args.eth_rpc_url,
        cc3_evm_private_key: args.cc3_evm_private_key,
        chain_key: args.chain_key,
        claim_buffer: args.claim_buffer,
        postgres_uri: args.postgres_uri,
        prover_be_socket_addr: args.prover_be_socket_addr,
        be_api_key: args.be_api_key,
    };

    let mut server = Server::new(config).await?;
    server.run().await?;

    // Wait for Ctrl+C signal
    signal::ctrl_c().await?;
    info!("Ctrl+C received, shutting down...");

    Ok(())
}
