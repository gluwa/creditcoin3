use clap::Parser;
use std::{error::Error, fs};
use tokio::signal;
use tracing::{debug, info};

use prover::{
    config::{ChainPriceConfigurations, Config},
    Server,
};

#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct Attestor {
    #[arg(long, default_value = "ws://localhost:9944")]
    cc3_rpc_url: String,

    #[arg(long, required = true)]
    cc3_key: String,

    #[arg(long, required = true)]
    nickname: String,

    #[arg(short, long)]
    verbose: bool,

    #[arg(long, default_value_t = 100)]
    claim_buffer: u8,

    #[arg(short, long, default_value = "./config.toml", required = true)]
    config_file: String,

    #[arg(
        long,
        default_value = "postgres://prover:prover@127.0.0.1:5432/attestations"
    )]
    postgres_uri: String,
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

    let config_file = fs::read_to_string(args.config_file)?;
    let chain_price_configurations: ChainPriceConfigurations = toml::from_str(&config_file)?;

    let config = Config {
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
        nickname: args.nickname,
        claim_buffer: args.claim_buffer,
        chain_price_configurations,
        postgres_uri: args.postgres_uri,
    };

    let mut server = Server::new(config)?;
    server.run().await?;

    // Wait for Ctrl+C signal
    signal::ctrl_c().await?;
    info!("Ctrl+C received, shutting down...");

    Ok(())
}
