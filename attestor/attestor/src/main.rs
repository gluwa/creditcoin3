use attestor::prelude::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

/// Configuration is read in hierarchically with the following order or priority:
///
/// 1. Cli args
/// 2. Env variables
/// 3. Config file (defaults to `config.yaml`)
struct Config {
    name: String,
    logs: std::path::PathBuf,
    secret: cc_client::secret::Secret,
    chain_key: attestor_primitives::ChainKey,
    public_addr: Option<String>,
    api_port: u16,
    boot_nodes: Vec<libp2p::Multiaddr>,
    p2p_port: u16, // Defaults to 9000 if not specified
    eth_url: url::Url,
    cc3_url: url::Url,
    pool_capacity: std::num::NonZeroUsize,
    start_height: Option<common::types::Height>,
    attestation_interval: Option<std::num::NonZero<common::types::Height>>,
    no_mdns: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFile {
    #[serde(default)]
    attestor: ConfigFileAttestor,
    #[serde(default)]
    api: ConfigFileApi,
    #[serde(default)]
    p2p: ConfigFileP2P,
    #[serde(default)]
    eth: ConfigFileEth,
    #[serde(default)]
    cc3: ConfigFileCC3,
    #[serde(default)]
    pool: ConfigFilePool,
    #[serde(default)]
    attestation: ConfigAttestation,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFileAttestor {
    name: Option<String>,
    chain_key: Option<attestor_primitives::ChainKey>,
    /// BIP39 mnemonic or raw 32-byte seed as hex (e.g. 0x398f...)
    secret: Option<cc_client::secret::Secret>,
    public_addr: Option<String>,
    #[serde(default = "default_logs")]
    logs: std::path::PathBuf,
}

fn default_logs() -> std::path::PathBuf {
    std::path::PathBuf::from("./logs")
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFileApi {
    port: Option<u16>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFileP2P {
    boot_nodes: Option<Vec<libp2p::Multiaddr>>,
    port: Option<u16>,
    #[serde(default)]
    no_mdns: bool,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFileEth {
    url: Option<url::Url>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFileCC3 {
    url: Option<url::Url>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigFilePool {
    capacity: Option<std::num::NonZeroUsize>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ConfigAttestation {
    start_height: Option<common::types::Height>,
    interval: Option<std::num::NonZero<common::types::Height>>,
}

impl Config {
    fn parse() -> anyhow::Result<Self> {
        // --------------------------------* Read config from file *-------------------------------

        let mut args = std::env::args();
        let mut config_path = std::env::var("ATTESTOR_CONFIG").ok();

        while let Some(arg) = args.next() {
            if arg.starts_with("--config") {
                if arg == "--config" {
                    config_path = Some(args.next().unwrap_or_default());
                } else {
                    config_path = Some(arg.trim_start_matches("--config=").to_string());
                }
            }
        }

        // We actually return an error if the user set a custom config file path but the file could
        // not be found or deserialization failed.
        //
        // If the user DID NOT provide any custom config file path then we do not care if the
        // default config file cannot be found. However, if a default config file IS present but
        // cannot be deserialized successfully, this also counts as an error.
        let config_file = match config_path {
            Some(path) => std::fs::File::open(path).map(serde_yaml::from_reader)??,
            None => std::fs::File::open("./config.yaml")
                .map(serde_yaml::from_reader)
                .unwrap_or(Ok(ConfigFile::default()))?,
        };

        // -------------------------------* Read config from cli/env *-----------------------------

        let matches = clap::command!()
            .arg(
                clap::arg!(-n --name <NAME>)
                    .help("Local attestors name")
                    .long_help("Local attestors name, used for display and debug purposes")
                    .env("ATTESTOR_NAME")
                    .required(config_file.attestor.name.is_none()),
            )
            .arg(
                clap::arg!(-k --"chain-key" <KEY>)
                    .help("Source chain to attest to")
                    .env("ATTESTOR_CHAIN_KEY")
                    .required(config_file.attestor.chain_key.is_none())
                    .value_parser(clap::value_parser!(attestor_primitives::ChainKey))
            )
            .arg(
                clap::arg!(-s --secret <SECRET>)
                    .help("Secret key: BIP39 mnemonic or 0x-prefixed 32-byte hex seed")
                    .long_help(
                        "Secret key used to sign attestation votes. \
                        Either a BIP39 mnemonic phrase or a raw 32-byte seed as hex (e.g. 0x398f...). \
                        If no key is provided a random mnemonic will be used instead",
                    )
                    .env("ATTESTOR_SECRET")
                    .required(false)
                    .value_parser(clap::value_parser!(cc_client::secret::Secret)),
            )
            .arg(
                clap::arg!(--"public-addr" <PORT>)
                    .help("P2P listening address")
                    .long_help(
                        "P2P listening address for libp2p networking. \
                        If not specified, a random OS-assigned ipv4 address will be used. \
                        Use this to set a fixed dns address for Kubernetes LoadBalancer services.",
                    )
                    .env("ATTESTOR_PUBLIC_ADDRESS")
                    .required(false)
                    .value_parser(clap::value_parser!(String)),
            )
            .arg(
                clap::arg!(--"api-port" <PORT>)
                    .help("Attestor api port")
                    .long_help(
                        "Attestor api port. \
                        Exposes a /metrics endpoints to query OpenTelemetry-style metrics \
                        summarizing the attestor's operational state."
                    )
                    .env("ATTESTOR_API_PORT")
                    .required(false)
                    .value_parser(clap::value_parser!(u16))
            )
            .arg(
                clap::arg!(-b --"boot-nodes" [MULTIADDR] ...)
                    .help("Existing nodes in the network")
                    .long_help(
                        "Existing nodes in the network. \
                        Used to establish a map of available peers",
                    )
                    .env("ATTESTOR_BOOT_NODES")
                    .action(clap::ArgAction::Append)
                    .required(false)
                    .value_parser(clap::value_parser!(libp2p::Multiaddr)),
            )
            .arg(
                clap::arg!(--"p2p-port" <PORT>)
                    .help("P2P listening port")
                    .long_help(
                        "P2P listening port for libp2p networking. \
                        If not specified, defaults to 9000. \
                        Specify a fixed port for Kubernetes LoadBalancer services.",
                    )
                    .env("ATTESTOR_P2P_PORT")
                    .required(false)
                    .value_parser(clap::value_parser!(u16)),
            )
            .arg(
                clap::arg!(--"no-mdns")
                    .help("Disable mDNS local peer discovery")
                    .long_help(
                        "Disable mDNS local peer discovery. \
                        mDNS is used for discovering peers on the local network. \
                        Disabling it has no effect on global peer discovery via Kademlia. \
                        This is useful in environments where mDNS is not supported (e.g. Kubernetes) \
                        or not desired.",
                    )
                    .env("ATTESTOR_NO_MDNS")
                    .required(false)
                    .action(clap::ArgAction::SetTrue),
            )
            .arg(
                clap::arg!(--"eth-url" <URL>)
                    .help("Eth RPC url")
                    .long_help(
                        "Eth RPC url. \
                        Used to pull source chain data and generate continuity proofs",
                    )
                    .env("ATTESTOR_ETH_URL")
                    .required(config_file.eth.url.is_none())
                    .value_parser(clap::value_parser!(url::Url)),
            )
            .arg(
                clap::arg!(--"cc3-url" <URL>)
                    .help("CC3 RPC url")
                    .long_help(
                        "CC3 RPC url. \
                        Used to listen to CC3 events and storage changes",
                    )
                    .env("ATTESTOR_CC3_URL")
                    .required(config_file.cc3.url.is_none())
                    .value_parser(clap::value_parser!(url::Url)),
            )
            .arg(
                clap::arg!(--"pool-capacity" <SIZE>)
                    .help("Maximum number of pending attestation")
                    .long_help(
                        "Maximum number of pending attestation. \
                        Once this count has been reached, the attestor will automatically start \
                        evicting attestations to make space for new votes",
                    )
                    .env("ATTESTOR_POOL_CAPACITY")
                    .required(config_file.pool.capacity.is_none())
                    .value_parser(clap::value_parser!(std::num::NonZeroUsize)),
            )
            .arg(
                clap::arg!(--"start-height" <HEIGHT>)
                    .help("Initial height from which the attestor starts producing attestations")
                    .long_help(
                        "Initial height from which the attestor starts producing attestations. \
                        If no starting height is specified, attestations will be generated from \
                        that chain's configured genesis block number instead"
                    )
                    .env("ATTESTOR_START_HEIGHT")
                    .required(false)
                    .value_parser(clap::value_parser!(common::types::Height)),
            )
            .arg(
                clap::arg!(--"attestation-interval" <INTERVAL>)
                    .help("Source chain block interval at which new attestations are produced")
                    .long_help(
                        "Source chain block interval at which attestations are produced. \
                        By default this value is fetched from on-chain storage, this options overrides it"
                    )
                    .env("ATTESTOR_ATTESTATION_INTERVAL")
                    .required(false)
                    .value_parser(clap::value_parser!(std::num::NonZero<common::types::Height>)),
            )
            .arg(
                clap::arg!(--logs <FOLDER> )
                    .help("Path to the logs folder")
                    .long_help(
                        "Path to the logs folder. Attestor logs will be saved there.",
                    )
                    .required(false)
                    .env("ATTESTOR_LOGS")
                    .value_parser(clap::value_parser!(std::path::PathBuf)),
            )
            .arg(
                clap::arg!(--config <FILE> )
                    .help("Path to the attestor config")
                    .long_help(
                        "Path to the attestor config, must point to a file in valid yaml syntax",
                    )
                    .required(false)
                    .default_value("./config.yaml")
                    .env("ATTESTOR_CONFIG")
                    .value_parser(clap::value_parser!(std::path::PathBuf)),
            )
            .get_matches();

        // ---------------------------------* Merge Configurations *-------------------------------

        // TODO: add some unit tests for this!

        let name = match matches.get_one::<String>("name") {
            Some(name) => name.to_string(),
            None => config_file
                .attestor
                .name
                .expect("Name is set either in config or by clap"),
        };

        let logs = match matches.get_one::<std::path::PathBuf>("logs") {
            Some(logs) => logs.clone(),
            None => config_file.attestor.logs,
        };

        let chain_key = match matches.get_one::<attestor_primitives::ChainKey>("chain-key") {
            Some(chain_key) => *chain_key,
            None => config_file
                .attestor
                .chain_key
                .expect("Chain key is set either in config or by clap"),
        };

        let secret = match matches.get_one::<cc_client::secret::Secret>("secret") {
            Some(secret) => secret.clone(),
            None => config_file.attestor.secret.unwrap_or(
                bip39::Mnemonic::generate(12)
                    .expect("Failed to generate attestor secret")
                    .into(),
            ),
        };

        let api_port = matches
            .get_one::<u16>("api-port")
            .cloned()
            .or(config_file.api.port)
            .unwrap_or(common::constants::DEFAULT_API_PORT);

        let boot_nodes = matches
            .get_one::<Vec<libp2p::Multiaddr>>("boot-nodes")
            .cloned()
            .or(config_file.p2p.boot_nodes)
            .unwrap_or_default();

        let public_addr = matches
            .get_one::<String>("public-addr")
            .cloned()
            .or(config_file.attestor.public_addr);

        let p2p_port = matches
            .get_one::<u16>("p2p-port")
            .copied()
            .or(config_file.p2p.port)
            .unwrap_or(common::constants::DEFAULT_P2P_PORT);

        let eth_url = match matches.get_one::<url::Url>("eth-url") {
            Some(url) => url.clone(),
            None => config_file
                .eth
                .url
                .expect("Eth url is set either in config or by clap"),
        };

        let cc3_url = match matches.get_one::<url::Url>("cc3-url") {
            Some(url) => url.clone(),
            None => config_file
                .cc3
                .url
                .expect("CC3 url is set either in config or by clap"),
        };

        let pool_capacity = match matches.get_one::<std::num::NonZeroUsize>("pool-capacity") {
            Some(pool_capacity) => *pool_capacity,
            None => config_file
                .pool
                .capacity
                .expect("Pool capacity is set either in config or by clap"),
        };

        let start_height = matches
            .get_one::<common::types::Height>("start-height")
            .cloned()
            .or(config_file.attestation.start_height);

        let attestation_interval = matches
            .get_one::<std::num::NonZero<common::types::Height>>("attestation-interval")
            .cloned()
            .or(config_file.attestation.interval);

        let no_mdns = matches.get_flag("no-mdns") || config_file.p2p.no_mdns;

        Ok(Config {
            name,
            logs,
            chain_key,
            secret,
            boot_nodes,
            public_addr,
            api_port,
            p2p_port,
            eth_url,
            cc3_url,
            pool_capacity,
            start_height,
            attestation_interval,
            no_mdns,
        })
    }
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    use tracing_subscriber::Layer as _;

    // ------------------------------------* User-facing logs *------------------------------------

    // If the user has set a RUST_LOG env variable, we use that as the default filter.
    // Otherwise, we default to info for our own crate and warnings for alloy and subxt.
    let filter_env = std::env::var("RUST_LOG")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|_| tracing_subscriber::EnvFilter::from_default_env())
        .unwrap_or_else(|| {
            tracing_subscriber::EnvFilter::new(
                "attestor=info,stream_attestation=info,stream_eth=info,alloy=warn,subxt=warn",
            )
        });

    let is_max_level_debug =
        filter_env.max_level_hint().unwrap() == tracing::level_filters::LevelFilter::DEBUG;
    let fmt = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_file(is_max_level_debug)
        .with_line_number(is_max_level_debug)
        .with_thread_ids(true)
        .with_filter(filter_env);

    let args = Config::parse()?;

    // -------------------------------------* Dev-facing logs *------------------------------------

    let filter_logs = tracing_subscriber::filter::Targets::new()
        .with_default(tracing_subscriber::filter::LevelFilter::OFF)
        .with_target("attestor", tracing::Level::TRACE)
        .with_target("stream_attestation", tracing::Level::TRACE)
        .with_target("stream_eth", tracing::Level::TRACE)
        .with_target("alloy", tracing::Level::WARN)
        .with_target("subxt", tracing::Level::DEBUG);
    let (appender, _guard) = tracing_appender::non_blocking(tracing_appender::rolling::hourly(
        args.logs,
        format!("attestor-{}.json", args.name),
    ));
    let logfile = tracing_subscriber::fmt::layer()
        .json()
        .with_target(true)
        .with_file(true)
        .with_line_number(true)
        .with_writer(appender)
        .with_filter(filter_logs);

    let _ = tracing_subscriber::registry()
        .with(fmt)
        .with(logfile)
        .try_init();

    // --------------------------------------* Configuration *-------------------------------------

    let config = attestor::ConfigBuilder::new()
        .with_name(args.name)
        .with_chain_key(args.chain_key)
        .with_secret(args.secret)
        .with_stream(
            attestor::stream_legacy::ConfigBuilder::new()
                .with_url_eth(args.eth_url)
                .with_url_cc3(args.cc3_url)
                .build(),
        )
        .with_p2p(
            attestor::worker::p2p::ConfigBuilder::new()
                .with_boot_nodes(args.boot_nodes)
                .with_public_addr(args.public_addr)
                .with_port(args.p2p_port)
                .with_no_mdns(args.no_mdns),
        )
        .with_pool(
            attestor::worker::validation::pool::ConfigBuilder::new()
                .with_max_size(args.pool_capacity),
        )
        .with_attestation(
            attestor::attestation::ConfigBuilder::new()
                .with_attestation_interval(args.attestation_interval)
                .with_start_height(args.start_height)
                .build(),
        )
        .with_api(attestor::worker::api::ConfigBuilder::new().with_port(args.api_port))
        .build();

    // ----------------------------------------* Main loop *---------------------------------------

    attestor::Attestor::new(config).run().await?;

    Ok(())
}
