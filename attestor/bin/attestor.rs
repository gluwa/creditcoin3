use clap::Parser;
use std::error::Error;
use tracing::debug;

use attestor::{Config, Server};
#[derive(Parser, Debug)]
#[command(name = "attestor")]
pub struct Attestor {
    #[arg(
        long,
        default_value = "ws://localhost:8545",
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

    #[arg(short, long, help = "Turn on verbose logging")]
    verbose: bool,

    #[arg(
        long,
        default_value = "2",
        help = "Chain key for the source chain, must match the chain key on creditcoin3"
    )]
    chain_key: u64,

    #[arg(
        long,
        default_value = "10",
        help = "Maturity delay for the source chain block to be considered final"
    )]
    maturity_delay: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Attestor::parse();

    // enable tracing debug logs if verbose flag is set
    let env_filter = if args.verbose {
        debug!("debug mode enabled!");
        "attestor=debug"
    } else {
        "attestor=info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(args.verbose)
        .with_env_filter(env_filter)
        .try_init();

    let config = Config {
        eth_rpc_url: args.eth_rpc_url,
        cc3_rpc_url: args.cc3_rpc_url,
        cc3_key: args.cc3_key,
        maturity_delay: args.maturity_delay,
        chain_key: args.chain_key,
        enable_prometheus_metrics: args.enable_prometheus_metrics,
        prometheus_host: args.prometheus_host,
        prometheus_port: args.prometheus_port,
    };

    let mut server = Server::new(config);

    server.run().await?;

    Ok(())
}
