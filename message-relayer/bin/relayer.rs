//! Message relayer binary entrypoint.
//!
//! Two operating modes:
//!  * `--config <path>` loads a multi-route YAML file (see `config.example.yaml`).
//!  * `--single-route` builds a one-route configuration from explicit CLI flags. Useful for
//!    local development against a single Anvil + a single Creditcoin node.

use std::path::PathBuf;
use std::str::FromStr;

use alloy::primitives::Address;
use anyhow::{bail, Context, Result};
use clap::Parser;
use message_relayer::config::{
    AttestorSet, ChainRoute, Config, P2pConfig, DEFAULT_BLOCK_CONFIRMATION_DEPTH, DEFAULT_P2P_PORT,
};
use message_relayer::Server;
use tracing::{debug, info};

#[derive(Parser, Debug)]
#[command(name = "message-relayer")]
struct Cli {
    /// Verbose tracing (`debug` level).
    #[arg(short, long)]
    verbose: bool,

    /// Multi-route YAML configuration. Cannot be combined with `--single-route`.
    #[arg(long, env = "RELAYER_CONFIG_FILE")]
    config: Option<PathBuf>,

    /// Build a one-route configuration from CLI flags. Cannot be combined with `--config`.
    #[arg(long, default_value_t = false)]
    single_route: bool,

    /// HTTP bind address for `/metrics` and `/health`.
    #[arg(long, default_value = "0.0.0.0", env = "RELAYER_BIND_HOST")]
    bind_host: String,

    /// HTTP bind port for `/metrics` and `/health`.
    #[arg(long, default_value_t = 3200, env = "RELAYER_BIND_PORT")]
    bind_port: u16,

    /// Creditcoin Substrate RPC (WebSocket). Used by future on-chain attestor resolution.
    #[arg(
        long,
        default_value = "ws://localhost:9944",
        env = "RELAYER_CC3_RPC_URL"
    )]
    cc3_rpc_url: String,

    /// Creditcoin EVM RPC (HTTP or WS). Required: outbox event polling reads from here.
    #[arg(
        long,
        default_value = "http://localhost:9933",
        env = "RELAYER_CC3_ETH_RPC_URL"
    )]
    creditcoin_eth_rpc_url: String,

    /// libp2p TCP port.
    #[arg(long, default_value_t = DEFAULT_P2P_PORT, env = "RELAYER_P2P_PORT")]
    p2p_port: u16,

    /// Disable mDNS discovery (recommended on Kubernetes).
    #[arg(long, default_value_t = false, env = "RELAYER_NO_MDNS")]
    no_mdns: bool,

    // ---------- single-route flags (only honoured with --single-route) -----------------------
    /// Creditcoin chain_key served by this route.
    #[arg(long, env = "RELAYER_CHAIN_KEY", required = false)]
    chain_key: Option<u64>,

    /// `eth_chainId` of the Creditcoin L1; bound for `messageHash`.
    #[arg(long, env = "RELAYER_CC3_CHAIN_ID", required = false)]
    cc3_chain_id: Option<u64>,

    /// Optional Outbox address override (else resolved from chain factory; PoC stub).
    #[arg(long, env = "RELAYER_OUTBOX_ADDRESS", required = false)]
    outbox_address: Option<String>,

    /// Destination chain RPC URL (HTTP or WS) for `Inbox.deliverMessage`.
    #[arg(long, env = "RELAYER_DESTINATION_RPC_URL", required = false)]
    destination_rpc_url: Option<String>,

    /// Inbox address on the destination chain.
    #[arg(long, env = "RELAYER_INBOX_ADDRESS", required = false)]
    inbox_address: Option<String>,

    /// Hex private key (with or without `0x`) for the destination signer wallet.
    #[arg(long, env = "RELAYER_SIGNER_KEY", required = false)]
    signer_key: Option<String>,

    /// Comma-separated EVM addresses of trusted attestors (static allowlist).
    #[arg(long, env = "RELAYER_ATTESTOR_SET", required = false)]
    attestor_set: Option<String>,

    /// Override the `2N/3 + 1` quorum threshold (development).
    #[arg(long, env = "RELAYER_THRESHOLD_OVERRIDE", required = false)]
    threshold_override: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let cli = Cli::parse();

    let env_filter = if cli.verbose {
        debug!("debug mode enabled");
        "debug"
    } else {
        "info"
    };

    let _ = tracing_subscriber::fmt()
        .compact()
        .with_file(false)
        .with_target(cli.verbose)
        .with_env_filter(env_filter)
        .try_init();

    let config = build_config(cli)?;

    let server = Server::new(config).await?;
    server.run().await?;
    info!("🛑 message relayer exited");
    Ok(())
}

fn build_config(cli: Cli) -> Result<Config> {
    if cli.config.is_some() && cli.single_route {
        bail!("--config and --single-route are mutually exclusive");
    }

    if let Some(path) = cli.config.clone() {
        return Config::from_yaml_file(&path, cli.cc3_rpc_url, cli.creditcoin_eth_rpc_url)
            .with_context(|| format!("failed to load relayer config from {}", path.display()));
    }

    if cli.single_route {
        return single_route_config(cli);
    }

    bail!("either --config <path> or --single-route must be supplied")
}

fn single_route_config(cli: Cli) -> Result<Config> {
    let chain_key = cli
        .chain_key
        .context("--chain-key is required for --single-route")?;
    let cc3_chain_id = cli
        .cc3_chain_id
        .context("--cc3-chain-id is required for --single-route")?;
    let destination_rpc_url = cli
        .destination_rpc_url
        .context("--destination-rpc-url is required for --single-route")?;
    let inbox_raw = cli
        .inbox_address
        .context("--inbox-address is required for --single-route")?;
    let attestor_csv = cli
        .attestor_set
        .context("--attestor-set is required for --single-route")?;

    let inbox_address = Address::from_str(inbox_raw.trim())
        .with_context(|| format!("invalid --inbox-address: {inbox_raw}"))?;
    let outbox_address = cli
        .outbox_address
        .as_deref()
        .map(|s| {
            Address::from_str(s.trim()).with_context(|| format!("invalid --outbox-address: {s}"))
        })
        .transpose()?;

    let attestor_addresses: Vec<Address> = attestor_csv
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| Address::from_str(s).with_context(|| format!("invalid attestor address: {s}")))
        .collect::<Result<Vec<_>>>()?;
    if attestor_addresses.is_empty() {
        bail!("--attestor-set must contain at least one EVM address");
    }

    let route = ChainRoute {
        chain_key,
        creditcoin_chain_id: cc3_chain_id,
        outbox_address,
        destination_rpc_url,
        inbox_address,
        signer_key: cli.signer_key,
        block_confirmation_depth: DEFAULT_BLOCK_CONFIRMATION_DEPTH,
        attestor_set: AttestorSet::Static(attestor_addresses),
        threshold_override: cli.threshold_override,
    };

    Ok(Config::single_route(
        cli.bind_host,
        cli.bind_port,
        cli.cc3_rpc_url,
        cli.creditcoin_eth_rpc_url,
        route,
        P2pConfig {
            port: cli.p2p_port,
            public_addr: None,
            boot_nodes: Vec::new(),
            no_mdns: cli.no_mdns,
            identity: None,
        },
    ))
}
