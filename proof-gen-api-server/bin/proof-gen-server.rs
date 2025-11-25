use clap::Parser;
use proof_gen_api_server::{config::Config, db::DbManager, Server};
use std::env;
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

    #[arg(
        long,
        default_value_t = false,
        help = "Enable deterministic mock RPC providers instead of real chain RPC endpoints"
    )]
    use_mock_providers: bool,
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
        "info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(args.verbose)
        .with_env_filter(env_filter)
        .try_init();

    // Resolve cc3_key: prefer CLI, fallback to env var for backward compatibility with README examples
    let resolved_cc3_key = args
        .cc3_key
        .or_else(|| env::var("CC3_KEY").ok())
        .unwrap_or_else(|| {
            eprintln!("Missing Creditcoin key: pass --cc3-key or set CC3_KEY env var");
            std::process::exit(1);
        });

    let mut config = Config {
        bind_addr: env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3100".to_string()),
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: resolved_cc3_key,
        chain_key: args.chain_key,
        eth_rpc_url: args.eth_rpc_url,
        use_mock_providers: false, // will override below from CLI
        enable_prometheus_metrics: args.enable_prometheus_metrics,
        prometheus_host: args.prometheus_host,
        prometheus_port: args.prometheus_port,
    };

    // Override mock provider selection from CLI flag
    config.use_mock_providers = args.use_mock_providers;

    if args.reset_db {
        info!("Resetting database...");
        manager.reset_db().await?;
        info!("Database reset successful");
        return Ok(());
    }

    let server = Server::new(config, manager).await?;
    // Run blocks until graceful shutdown signal.
    server.run().await?;
    info!("🛑 Server exited");

    Ok(())
}
