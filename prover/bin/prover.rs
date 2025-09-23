use clap::Parser;
use dotenv::dotenv;
use prover::{config::Config, Server};
use std::error::Error;
use tokio::signal;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(name = "prover")]
#[allow(clippy::struct_excessive_bools)]
pub struct Prover {
    #[arg(long, default_value = "ws://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_key: String,

    #[arg(
        long,
        default_value = "2",
        help = "Chain key for the source chain, must match the chain key on creditcoin3"
    )]
    chain_key: u64,

    #[arg(long, default_value = "ws://localhost:8545")]
    eth_rpc_url: String,

    #[arg(long, required = true, env)]
    cc3_evm_private_key: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long, help = "Show Cairo logs without enabling verbose mode")]
    enable_cairo_logs: bool,

    #[arg(long, required = false, default_value_t = 10)]
    cost_per_byte: u64,

    #[arg(long, required = false, default_value_t = 1000)]
    base_fee: u64,

    #[arg(long, default_value_t = 100)]
    claim_buffer: u8,

    #[arg(long, required = false, env)]
    postgres_uri: String,

    #[arg(long, required = false)]
    prover_be_socket_addr: Option<String>,

    #[arg(long, required = false, env)]
    be_api_key: Option<String>,

    #[arg(long, required = true)]
    name: String,

    #[arg(
        short,
        long,
        default_value_t = 1800,
        help = "Timeout in seconds for the prover to submit the query proof back to the contract, default value = 30 minutes (1800 seconds)"
    )]
    timeout: u64,

    #[arg(long, help = "Reset the database to its initial state")]
    reset_db: bool,

    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "Bind address for the prometheus metrics server."
    )]
    prometheus_host: String,

    #[arg(
        long,
        required = false,
        default_value_t = 9100,
        help = "Port to expose the Prometheus metrics endpoint on. Defaults to 9100."
    )]
    prometheus_port: u16,

    #[arg(
        long,
        required = false,
        help = "Flag indicating the attestor will launch a server to expose metrics."
    )]
    enable_prometheus_metrics: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize environment file
    dotenv().ok();

    // Parse args
    let args = Prover::parse();

    // Propagate Cairo logs preference to child processes (Python scripts)
    // This allows enabling Cairo logs without enabling full verbose mode.
    if args.enable_cairo_logs {
        std::env::set_var("CAIRO_LOGS", "true");
    } else {
        std::env::set_var("CAIRO_LOGS", "false");
    }

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
        .with_target(args.verbose)
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
        chain_key: args.chain_key,
        eth_rpc_url: args.eth_rpc_url,
        cc3_evm_private_key: args.cc3_evm_private_key,
        cost_per_byte: args.cost_per_byte,
        base_fee: args.base_fee,
        claim_buffer: args.claim_buffer,
        postgres_uri: args.postgres_uri.clone(),
        prover_be_socket_addr: args.prover_be_socket_addr,
        be_api_key: args.be_api_key,
        name: args.name,
        timeout: args.timeout,
        prometheus_host: args.prometheus_host,
        prometheus_port: args.prometheus_port,
        enable_prometheus_metrics: args.enable_prometheus_metrics,
    };

    if args.reset_db {
        info!("Resetting database...");
        prover::postgres::db::reset_database(args.postgres_uri).await?;
        info!("Database reset successful");
        return Ok(());
    }

    let mut server = Server::new(config).await?;
    server.run().await?;

    // Wait for Ctrl+C signal
    signal::ctrl_c().await?;
    info!("🛑 Ctrl+C received, shutting down...");

    Ok(())
}
