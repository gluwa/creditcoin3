use attestor::prelude::*;

// These seeds where obtained by running `creditcoin3-node key inspect <ACCOUNT_URI>` for each of the first 6 Substrate development accounts
// (//Alice, //Bob, //Charlie, //Dave, //Eve, //Ferdie).
const WELL_KNOWN_SEEDS: [&str; 6] = [
    "0xe5be9a5092b81bca64be81d212e7f2f9eba183bb7a90954f7b76361f6edb5c0a", // //Alice
    "0x398f0c28f98885e046333d4a41c19cee4c37368a9832c6502f6cfd182e2aef89", // //Bob
    "0xbc1ede780f784bb6991a585e4f6e61522c14e1cae6ad0895fb57b9a205a8f938", // //Charlie
    "0x868020ae0687dda7d57565093a69090211449845a7e11453612800b663307246", // //Dave
    "0x786ad0e2df456fe43dd1f91ebca22e235bc162e0bb8d53c633e8c85b2af68b7a", // //Eve
    "0x42438b7883391c05512a938e36c2df0131e088b3756d6aa7a755fbff19d2f842", // //Ferdie
];

#[derive(Debug, clap::Parser)]
struct Args {
    /// Number of attestors to spawn
    #[arg(long, short)]
    number: std::num::NonZeroUsize,

    /// Starts numbering attestors as of
    #[arg(long, default_value("0"))]
    offset: usize,

    /// Path to the attestor binary
    #[arg(long)]
    bin: std::path::PathBuf,

    /// Source chain to attest to, defaults to Ethereum
    #[arg(long, default_value("2"))]
    chain_key: attestor_primitives::ChainKey,

    /// Ethereum WS RPC url
    #[arg(long, default_value_t = url::Url::parse("ws://localhost:8545").unwrap())]
    eth_url: url::Url,

    /// Creditcoin WS RPC url
    #[arg(long, default_value_t = url::Url::parse("ws://localhost:9944").unwrap())]
    cc3_url: url::Url,

    /// If true, the program will fetch the current block number of the source chain and configure
    /// that as a genesis block for the attestors.
    ///
    /// This is only intended for testing purposes.
    #[arg(long)]
    configure_genesis: bool,

    /// Mnemonic for a creditcoin3 account that will fund the attestors
    #[arg(long)]
    funding_address: String,

    /// Base configuration shared among all attestors
    #[arg(long, default_value = "./config.yaml")]
    config: std::path::PathBuf,

    /// Base P2P port for the first attestor. Each subsequent attestor will use base_port + index.
    /// If not specified, defaults to 9000 (attestor 0 gets 9000, attestor 1 gets 9001, etc.).
    #[arg(long, default_value_t = attestor::common::constants::DEFAULT_P2P_PORT)]
    p2p_port_base: u16,

    /// Base API port for the first attestor. Each subsequent attestor will use base_port + index.
    /// If not specified, defaults to 9100 (attestor 0 gets 9100, attestor 1 gets 9101, etc.).
    #[arg(long, default_value_t = attestor::common::constants::DEFAULT_API_PORT)]
    api_port_base: u16,

    /// If true, the program will use well-known seeds for the attestors instead of generating them randomly.
    /// The well-known seeds are the secret seeds of the first 6 Substrate development accounts (//Alice, //Bob, //Charlie, //Dave, //Eve, //Ferdie).
    /// and are used in order for each attestor (first attestor gets the first seed, second attestor gets the second seed, etc.).
    #[arg(long)]
    well_known_keys: bool,

    /// Optional path to a file containing well-known mnemonics (one per line).
    /// If provided, these seeds are used instead of random generation.
    /// The seeds on the file are used in order, so the first line corresponds to the first attestor, the second line to the second attestor, and so on.
    /// The seeds from the file are used before the seeds from --seeds (if provided).
    /// If the combined list of seeds from the file and --seeds contains fewer seeds than the number of attestors to spawn,
    /// the remaining attestors's seeds will be generated randomly.
    #[arg(long)]
    seeds_file: Option<std::path::PathBuf>,

    #[arg(last = true)]
    trailing: Vec<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    use anyhow::Context as _;
    use chrono::{Datelike as _, Timelike as _};
    use clap::Parser as _;
    use rand::SeedableRng as _;
    use std::str::FromStr as _;
    use subxt_signer::ExposeSecret as _;

    const MAX_ATTEMPTS: usize = 10;

    // ------------------------------------* User-facing logs *------------------------------------

    let filter_env = tracing_subscriber::EnvFilter::builder()
        .with_default_directive("attestor_zombienet=info".parse().unwrap())
        .from_env_lossy();

    let debug = filter_env.max_level_hint().unwrap() == tracing::level_filters::LevelFilter::DEBUG;
    let _ = tracing_subscriber::fmt()
        .with_target(debug)
        .with_file(debug)
        .with_line_number(debug)
        .with_thread_ids(debug)
        .with_env_filter(filter_env)
        .try_init();

    // --------------------------------------* CLI arguments *-------------------------------------

    let args = Args::parse();

    let mut rng = rand::rngs::StdRng::seed_from_u64(42 + args.offset as u64);

    anyhow::ensure!(
        args.bin.as_path().exists(),
        "Failed to find attestor binary"
    );
    anyhow::ensure!(
        args.config.as_path().exists(),
        "Failed to find attestor config"
    );

    // Collect seeds from --seeds-file and --seeds
    let mut configured_secrets: Vec<(subxt_signer::SecretUri, String)> = Vec::new();

    if args.well_known_keys {
        for key in WELL_KNOWN_SEEDS.iter() {
            let secret_uri = subxt_signer::SecretUri::from_str(key).context(format!(
                "Failed to create secret uri from well-known key: {key}"
            ))?;
            let secret = secret_uri.phrase.expose_secret().to_string();
            configured_secrets.push((secret_uri, secret));
        }
    }

    if let Some(ref seeds_file) = args.seeds_file {
        let contents = std::fs::read_to_string(seeds_file).context(format!(
            "Failed to read seeds file: {}",
            seeds_file.display()
        ))?;
        for (i, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mnemonic: bip39::Mnemonic = line
                .parse()
                .context(format!("Invalid mnemonic on line {} of seeds file", i + 1))?;
            let secret_uri = subxt_signer::SecretUri::from_str(&mnemonic.to_string())
                .context("Failed to create secret uri")?;

            configured_secrets.push((secret_uri, mnemonic.to_string()));
        }
    }

    let attestor_count = args.number.get();
    // If more seeds are provided than attestors, truncate the list of seeds to match the number of attestors.
    configured_secrets.truncate(attestor_count);
    // Reverse to pop seeds in the original order
    configured_secrets.reverse();

    if !configured_secrets.is_empty() {
        tracing::info!(
            count = configured_secrets.len(),
            "🔑 Using configured secrets for attestors"
        );
    }

    let attestor_info = (0..attestor_count)
        .map(|mut n| {
            n += args.offset;
            let name = format!("zombie-{n}");

            let (secret_uri, secret) = if let Some(configured_secret) = configured_secrets.pop() {
                configured_secret
            } else {
                let mnemonic =
                    bip39::Mnemonic::generate_in_with(&mut rng, bip39::Language::English, 12)
                        .expect("Failed to generate attestor secret");
                let secret_uri = subxt_signer::SecretUri::from_str(&mnemonic.to_string())
                    .context("Failed to create secret uri")?;
                (secret_uri, mnemonic.to_string())
            };
            let keypair = subxt_signer::sr25519::Keypair::from_uri(&secret_uri)
                .context("Failed to create secret keypair")?;
            let account_id = cc_client::AccountId32(keypair.public_key().0);

            anyhow::Ok((name, secret, account_id))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // ------------------------------------* Connecting to CC3 *-----------------------------------

    loop {
        tokio::select! {
            Ok(_) = tokio_tungstenite::connect_async(&args.cc3_url) => {
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                return anyhow::Ok(());
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }
        tracing::info!(
            url = %args.cc3_url,
            "🛜  waiting for CC3 WS connection to be made available..."
        );
    }

    let cc3 = cc_client::Client::new(args.cc3_url.clone(), &args.funding_address)
        .await
        .context("Failed to initialize CC3 client")?;
    let cc3 = std::sync::Arc::new(cc3);

    let nonce = cc3
        .get_account_nonce()
        .await
        .context("Failed to get funding address nonce")?;
    let nonce = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(nonce));

    // ------------------------------------* Connecting to Eth *-----------------------------------

    loop {
        tokio::select! {
            Ok(_) = tokio_tungstenite::connect_async(&args.eth_url) => {
                break;
            }
            _ = tokio::signal::ctrl_c() => {
                return anyhow::Ok(());
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
        }
        tracing::info!(
            url = %args.eth_url,
            "🛜  waiting for Eth WS connection to be made available..."
        );
    }

    let eth = eth::Client::new(args.eth_url.as_ref(), None)
        .await
        .context("Failed to initialized eth client")?;

    // -----------------------------------------* Genesis *----------------------------------------

    if args.configure_genesis {
        let current_block = eth
            .get_last_block()
            .await
            .context("Failed to retrieve latest eth block")?;
        let attestation_interval = cc3
            .chain_attestation_interval(args.chain_key)
            .await
            .context("Failed to retrieve attestation chain interval")?
            .context("Invalid chain key")?;
        let start_block = current_block - (current_block % attestation_interval);

        tracing::info!(start_block, "👷 Configuring genesis block for attestors");

        let nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        cc3.set_attestation_chain_genesis_block_number(
            Some(nonce_local),
            args.chain_key,
            start_block,
        )
        .await
        .context("Failed to set chain genesis block")?;

        tracing::info!(
            chain_key = args.chain_key,
            "👷   Successfully set chain genesis block"
        );
    }

    // ------------------------------------* Attestor funding *------------------------------------

    tracing::info!("💵 Funding attestors");

    let mut futures_funding = tokio::task::JoinSet::new();
    for (name, _secret, account_id) in attestor_info.iter() {
        let name = name.clone();
        let account_id = account_id.clone();

        let cc3 = std::sync::Arc::clone(&cc3);
        let nonce = std::sync::Arc::clone(&nonce);

        let mut attempt = 0;

        futures_funding.spawn(async move {
            let mut nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            while let Err(err) = cc3
                .set_balance(
                    account_id.clone(),
                    10_000_000_000_000_000_000_000,
                    Some(nonce_local),
                )
                .await
            {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    anyhow::bail!("Failed to fun attestor {name} - {account_id}: {err}");
                }

                nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
            tracing::debug!(nonce_local, "OK - funding");
            anyhow::Ok((name, account_id))
        });
    }

    while let Some(res) = futures_funding.join_next().await {
        let (name, account_id) = res??;
        tracing::info!(name, %account_id, "💵   Successfully set attestor balance to 10 000 dev CTC");
    }

    // -----------------------------------* Attestor registration *--------------------------------

    tracing::info!("👷 Registering attestors");

    let blocks = cc3.api().blocks().subscribe_finalized().await.unwrap();

    let mut futures_register = tokio::task::JoinSet::new();
    for (name, _secret, account_id) in attestor_info.iter() {
        let cc3 = std::sync::Arc::clone(&cc3);
        let nonce = std::sync::Arc::clone(&nonce);

        let name = name.clone();
        let account_id = account_id.clone();

        let mut attempt = 0;

        futures_register.spawn(async move {
            let query = cc_client::cc3::storage()
                .attestation()
                .attestors(args.chain_key, &account_id);
            let attestor = cc3
                .api()
                .storage()
                .at_latest()
                .await
                .unwrap()
                .fetch(&query)
                .await
                .unwrap();

            if attestor.is_none() {
                let mut nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);

                while let Err(err) = cc3
                    .attestor_register(args.chain_key, account_id.clone(), Some(nonce_local))
                    .await
                {
                    tracing::error!(%err, "Failed to register attestor");

                    attempt += 1;
                    if attempt >= MAX_ATTEMPTS {
                        anyhow::bail!("Failed to register attestor {name} - {account_id}: {err}");
                    }

                    nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                }
                tracing::debug!(nonce_local, "OK - register");
                anyhow::Ok(Some((name, account_id)))
            } else {
                tracing::info!(name, %account_id, "👷   Already registered");
                anyhow::Ok(None)
            }
        });
    }

    let mut to_register = 0;
    while let Some(res) = futures_register.join_next().await {
        if let Some((name, account_id)) = res?? {
            tracing::info!(name, %account_id, "👷   Successfully registered attestor");
            to_register += 1;
        }
    }

    // ----------------------* Attestor registration (on-chain confirmation) *---------------------

    if to_register > 0 {
        wait_for_event::<cc_client::cc3::attestation::events::AttestorRegistered>(
            to_register,
            blocks,
        )
        .await?;
    }

    // -------------------------------------* Start attestors *------------------------------------

    let mut futures_attestors = tokio::task::JoinSet::new();
    for (index, (name, secret, account_id)) in attestor_info.iter().enumerate() {
        // Assign unique P2P port for each attestor
        let port_p2p = args.p2p_port_base + index as u16 + args.offset as u16;
        let port_api = args.api_port_base + index as u16 + args.offset as u16;

        let mut attestor = tokio::process::Command::new(&args.bin);
        attestor
            .kill_on_drop(true)
            .arg(format!("--name={name}"))
            .arg(format!("--secret={secret}"))
            .arg(format!("--config={}", args.config.to_string_lossy()))
            .arg(format!("--chain-key={}", args.chain_key))
            .arg(format!("--eth-url={}", args.eth_url))
            .arg(format!("--cc3-url={}", args.cc3_url))
            .arg(format!("--p2p-port={port_p2p}"))
            .arg(format!("--api-port={port_api}"));

        attestor
            .args(&args.trailing)
            .stdout(std::process::Stdio::null());

        let name = name.clone();
        let account_id = account_id.clone();

        futures_attestors.spawn(async move {
            let time = chrono::Utc::now();
            let year = time.year();
            let month = time.month();
            let day = time.day();
            let hour = time.hour();
            let logs = format!("logs/attestor-{name}.json.{year:04}-{month:02}-{day:02}-{hour:02}");

            tracing::info!(name, %account_id, "🏁 Starting attestor");
            tracing::info!(logs, "🏁   with");

            attestor
                .spawn()
                .context("Failed to start attestor")?
                .wait()
                .await
                .context("Failed to start attestor")?;
            anyhow::Ok((name, account_id))
        });
    }

    // ----------------------------------------* Shutdown *----------------------------------------

    while !futures_attestors.is_empty() {
        tokio::select! {
            biased;

            Some(res) = futures_attestors.join_next() => {
                match res {
                    Ok(Ok((name, account_id))) => {
                        tracing::info!(name, %account_id, "🔌 Attestor has shut down");
                    }
                    Ok(Err(err)) => tracing::error!(?err, "⛔ Attestor error"),
                    Err(err) => tracing::error!(?err, "⛔ Join error"),
                }
            }
            _ = tokio::signal::ctrl_c() => {}
        }
    }

    // ------------------------------------* Attestor chilling *-----------------------------------

    let nonce = cc3
        .get_account_nonce()
        .await
        .context("Failed to get funding address nonce")?;
    let nonce = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(nonce));

    tracing::info!("❄️ Chilling attestors");

    let blocks = cc3.api().blocks().subscribe_finalized().await.unwrap();

    let mut futures_chill = tokio::task::JoinSet::new();
    for (name, _secret, account_id) in attestor_info.iter() {
        let cc3 = std::sync::Arc::clone(&cc3);
        let nonce = std::sync::Arc::clone(&nonce);

        let name = name.clone();
        let account_id = account_id.clone();

        let mut attempt = 0;

        futures_chill.spawn(async move {
            let mut nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            while let Err(err) = cc3
                .attestor_chill(args.chain_key, account_id.clone(), Some(nonce_local))
                .await
            {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    anyhow::bail!("Failed to chill attestor {name} - {account_id}: {err}");
                }

                nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }

            tracing::debug!(nonce_local, "OK - chill");
            anyhow::Ok((name, account_id))
        });
    }

    while !futures_chill.is_empty() {
        tokio::select! {
            biased;

            Some(res) = futures_chill.join_next() => {
                let (name, account_id) = res??;
                tracing::info!(name, %account_id, "❄️   Successfully chilled attestor");
            }
            _ = tokio::signal::ctrl_c() => {
                return anyhow::Ok(());
            }
        }
    }

    // ------------------------* Attestor chilling (on-chain confirmation) *-----------------------

    wait_for_event::<cc_client::cc3::attestation::events::AttestorChilled>(attestor_count, blocks)
        .await?;

    // --------------------------------* Attestor un-registration *--------------------------------

    tracing::info!("🪦 Un-registering attestors");

    let mut futures_unregister = tokio::task::JoinSet::new();
    for (name, _secret, account_id) in attestor_info.iter() {
        let cc3 = std::sync::Arc::clone(&cc3);
        let nonce = std::sync::Arc::clone(&nonce);

        let name = name.clone();
        let account_id = account_id.clone();

        let mut attempt = 0;

        futures_unregister.spawn(async move {
            let mut nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            while let Err(err) = cc3
                .attestor_unregister(args.chain_key, account_id.clone(), Some(nonce_local))
                .await
            {
                attempt += 1;
                if attempt >= MAX_ATTEMPTS {
                    anyhow::bail!("Failed to un-register attestor {name} - {account_id}: {err}");
                }

                nonce_local = nonce.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }

            tracing::debug!(nonce_local, "OK - unregister");
            anyhow::Ok((name, account_id))
        });
    }

    while !futures_unregister.is_empty() {
        tokio::select! {
            biased;

            Some(res) = futures_unregister.join_next() => {
                let (name, account_id) = res??;
                tracing::info!(name, %account_id, "🪦   Successfully un-registered attestor");
            }
            _ = tokio::signal::ctrl_c() => {
                return anyhow::Ok(());
            }
        }
    }

    anyhow::Ok(())
}

async fn wait_for_event<Event>(
    mut count: usize,
    mut blocks: common::types::SubxtBlockStream,
) -> anyhow::Result<()>
where
    Event: subxt::events::StaticEvent,
{
    use anyhow::Context as _;

    'outer: loop {
        // NOTE: Cancellation
        //
        // Potentially long network round trips, manually handle cancellation to keep the program
        // responsive.
        let block = tokio::select! {
            biased;

            _ = tokio::signal::ctrl_c() => {
                return anyhow::Ok(());
            }
            block = blocks.next() => {
                block
                    .transpose()
                    .context("Failed to get next block")?
                    .context("Block stream ended unexpectedly")?
            }
        };

        // NOTE: Cancellation
        //
        // Potentially long network round trips, manually handle cancellation to keep the program
        // responsive.
        let events = tokio::select! {
            biased;

            _ = tokio::signal::ctrl_c() => {
                return anyhow::Ok(());
            }
            events = block.events() => {
                events.context("Failed to retrieve block events")?
            }
        };

        for event in events.iter() {
            let event = event.context("Failed to get next block events")?;

            if (Event::PALLET, Event::EVENT) == (event.pallet_name(), event.variant_name()) {
                tracing::debug!("Observed event");

                count = count.saturating_sub(1);
                if count == 0 {
                    break 'outer;
                }
            }
        }
    }

    Ok(())
}
