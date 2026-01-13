use clap::Parser;
use std::env;
use tracing::{debug, info};

use proof_gen_api_server::{config::Config, db::DbManager, Server};

#[derive(Parser, Debug)]
#[command(name = "proof-gen-api-server")]
pub struct ProofGenApiServer {
    #[arg(short, long)]
    verbose: bool,

    #[arg(long, help = "Reset the database to its initial state")]
    reset_db: bool,

    #[arg(long, default_value = "ws://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(
        long,
        required = false,
        help = "Creditcoin3 mnemonic/seed. If omitted, falls back to CC3_KEY env var."
    )]
    cc3_key: Option<String>,

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
        help = "Flag indicating the attestor will launch a server to expose metrics."
    )]
    enable_prometheus_metrics: bool,

    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "IP address for the prometheus metrics server (e.g., '0.0.0.0', '::1')"
    )]
    prometheus_host: String,

    #[arg(
        long,
        default_value_t = 9100,
        help = "Port to expose the Prometheus metrics endpoint on. Defaults to 9100."
    )]
    prometheus_port: u16,

    #[arg(
        long,
        default_value = "0.0.0.0",
        help = "IP address which the proof gen server binds to for API requests (e.g., '0.0.0.0', '::1')"
    )]
    bind_host: String,

    #[arg(
        long,
        default_value_t = 3100,
        help = "Port which the proof gen server binds to for API requests"
    )]
    bind_port: u16,

    #[arg(
        long,
        default_value = "postgres://postgres:password@localhost:5433/proofs_db",
        help = "The connection string used to access the server's postgres DB"
    )]
    postgres_uri: String,

    #[arg(
        long,
        required = false,
        help = "Redis connection URL for block caching layer"
    )]
    redis_url: Option<String>,

    #[arg(
        long,
        required = false,
        help = "CC3 Indexer GraphQL URL for pre-fetching continuity proofs"
    )]
    indexer_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env
    dotenvy::dotenv().ok();

    // Parse args
    let args = ProofGenApiServer::parse();

    // Get db connection details from env variables.
    let manager = DbManager::new(args.postgres_uri)?;

    // enable tracing debug logs if verbose flag is set
    let env_filter = if args.verbose {
        debug!("debug mode enabled!");
        "debug"
    } else {
        "info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(args.verbose)
        .with_env_filter(env_filter)
        .try_init();

    // cc3_key is optional - not needed for read-only operations
    // Prefer CLI, fallback to env var for backward compatibility
    let resolved_cc3_key = args.cc3_key.or_else(|| env::var("CC3_KEY").ok());

    let config = Config {
        bind_host: args.bind_host,
        bind_port: args.bind_port,
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: resolved_cc3_key,
        chain_key: args.chain_key,
        eth_rpc_url: args.eth_rpc_url,
        enable_prometheus_metrics: args.enable_prometheus_metrics,
        prometheus_host: args.prometheus_host,
        prometheus_port: args.prometheus_port,
        redis_url: args.redis_url,
        indexer_url: args.indexer_url,
    };

    if args.reset_db {
        info!("Resetting database...");
        manager.reset_database().await?;
        info!("Database reset successful");
        return Ok(());
    }

    let server = Server::new(config, manager).await?;
    // Run blocks until graceful shutdown signal.
    server.run().await?;
    info!("🛑 Server exited");

    Ok(())
}
