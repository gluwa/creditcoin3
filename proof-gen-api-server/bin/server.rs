use clap::Parser;
use std::env;
use std::path::PathBuf;
use tracing::{debug, info};

use proof_gen_api_server::config::{ChainConfig, Config};
use proof_gen_api_server::Server;

#[derive(Parser, Debug)]
#[command(name = "proof-gen-api-server")]
pub struct ProofGenApiServer {
    #[arg(short, long)]
    verbose: bool,

    #[arg(long, help = "Reset the database to its initial state")]
    reset_db: bool,

    /// Load multi-chain YAML configuration (see `config.example.yaml`).
    /// May also be set via `PROOF_GEN_CONFIG_FILE`.
    #[arg(long, env = "PROOF_GEN_CONFIG_FILE")]
    config: Option<PathBuf>,

    /// Creditcoin3 RPC (WebSocket). Use `CC3_RPC_URL` in `.env` or `--cc3-rpc-url` (not in YAML).
    #[arg(long, default_value = "ws://localhost:9944", env = "CC3_RPC_URL")]
    cc3_rpc_url: String,

    #[arg(
        long,
        required = false,
        help = "Creditcoin3 mnemonic/seed. If omitted, falls back to CC3_KEY env var."
    )]
    cc3_key: Option<String>,

    #[arg(long, default_value = "ws://localhost:8545")]
    eth_rpc_url: String,

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
        required = false,
        help = "Redis connection URL for block caching layer"
    )]
    redis_url: Option<String>,

    #[arg(
        long,
        default_value_t = false,
        help = "Use Redis Cluster client (required when Redis is in cluster mode, e.g. ElastiCache)"
    )]
    redis_cluster_mode: bool,

    #[arg(
        long,
        required = false,
        help = "CC3 Indexer GraphQL URL for pre-fetching continuity proofs"
    )]
    indexer_url: Option<String>,

    #[arg(
        long,
        default_value_t = 10,
        env = "MAX_BATCH_SIZE",
        help = "Maximum amount of concurrent futures spawned when generating proofs for batch requests or when extracting transaction indexes from transaction hashes. Adjust based on expected load and RPC rate limits."
    )]
    max_batch_size: usize,

    #[arg(
        long,
        required = false,
        env = "ARCHIVER_URL",
        help = "Archiver HTTP URL for the legacy single-chain mode (e.g. http://localhost:8080)."
    )]
    archiver_url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();

    let args = ProofGenApiServer::parse();

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

    let resolved_cc3_key = args.cc3_key.or_else(|| env::var("CC3_KEY").ok());
    let resolved_redis_url = args.redis_url.or_else(|| env::var("REDIS_URL").ok());
    let resolved_redis_cluster_mode = args.redis_cluster_mode
        || env::var("REDIS_CLUSTER_MODE")
            .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false);

    let mut config = if let Some(path) = args.config.clone() {
        let mut c = Config::from_yaml_file(&path, args.cc3_rpc_url.clone())?;
        if c.cc3_key.is_none() {
            c.cc3_key = resolved_cc3_key;
        }
        c
    } else {
        if args.max_batch_size == 0 {
            panic!("max_batch_size must be greater than 0");
        }
        // Legacy single-chain mode: chain key from CHAIN_KEY env (default 2).
        let chain_key = env::var("CHAIN_KEY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        Config {
            bind_host: args.bind_host,
            bind_port: args.bind_port,
            cc3_rpc_url: args.cc3_rpc_url,
            cc3_key: resolved_cc3_key,
            chains: vec![ChainConfig {
                chain_key,
                eth_rpc_url: args.eth_rpc_url,
                archiver_url: args.archiver_url,
            }],
            redis_url: resolved_redis_url,
            redis_cluster_mode: resolved_redis_cluster_mode,
            indexer_url: args.indexer_url,
            max_batch_size: args.max_batch_size,
        }
    };

    // CC3_RPC_URL in the environment overrides `--cc3-rpc-url` / default (same pattern as CC3_KEY).
    if let Ok(url) = env::var("CC3_RPC_URL") {
        let trimmed = url.trim();
        if !trimmed.is_empty() {
            config.cc3_rpc_url = trimmed.to_string();
        }
    }

    if config.max_batch_size == 0 {
        panic!("max_batch_size must be greater than 0");
    }

    let server = Server::new(config).await?;
    server.run().await?;
    info!("🛑 Server exited");

    Ok(())
}
