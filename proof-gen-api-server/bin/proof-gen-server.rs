use clap::Parser;
use proof_gen_api_server::{config::Config, db::DbManager, Server};
use tokio::signal;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(name = "proof-gen-api-server")]
pub struct ProofGenApiServer {
    #[arg(short, long, required = false)]
    verbose: bool,

    #[arg(
        long,
        required = false,
        help = "Reset the database to its initial state"
    )]
    reset_db: bool,

    #[arg(long, default_value = "ws://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(
        long,
        default_value = "2",
        help = "Chain key for the source chain, must match the chain key on creditcoin3"
    )]
    chain_key: u64,

    #[arg(long, default_value = "ws://localhost:8545")]
    eth_rpc_url: String,

    #[arg(
        long,
        required = false,
        help = "Flag indicating the attestor will launch a server to expose metrics."
    )]
    enable_prometheus_metrics: bool,

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
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env
    dotenvy::dotenv().ok();

    let manager = DbManager::new()?;

    // Parse args
    let args = ProofGenApiServer::parse();

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

    let config = Config {
        cc3_rpc_url: args.cc3_rpc_url,
        chain_key: args.chain_key,
        eth_rpc_url: args.eth_rpc_url,
        enable_prometheus_metrics: args.enable_prometheus_metrics,
        prometheus_host: args.prometheus_host,
        prometheus_port: args.prometheus_port,
    };

    if args.reset_db {
        info!("Resetting database...");
        manager.reset_db().await?;
        info!("Database reset successful");
        return Ok(());
    }

    let mut server = Server::new(config, manager).await?;
    server.run().await?;

    // Wait for Ctrl+C signal
    signal::ctrl_c().await?;
    info!("🛑 Ctrl+C received, shutting down...");

    Ok(())
}
