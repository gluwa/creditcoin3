#[derive(clap::Parser)]
struct Args {
    #[arg(long, default_value_t = url::Url::parse("ws://localhost:8545").unwrap())]
    eth_url: url::Url,

    #[arg(long, default_value_t = url::Url::parse("ws://localhost:9944").unwrap())]
    cc3_url: url::Url,

    #[arg(long, default_value_t = 0)]
    start_height: attestor_primitives::Height,

    #[arg(long, default_value_t = std::num::NonZeroUsize::new(1000).unwrap())]
    blocks: std::num::NonZeroUsize,
}

const FINALIZATION_LAG: attestor_primitives::Height = 10;
const INTERVAL_ATTESTATION: std::num::NonZeroU64 = std::num::NonZero::new(10).unwrap();
const MAX_CONCURRENT_REQUESTS: std::num::NonZeroUsize = std::num::NonZeroUsize::new(10).unwrap();
const MAX_CATCHUP: std::num::NonZeroU64 = std::num::NonZeroU64::new(50).unwrap();

fn main() {
    use clap::Parser as _;
    use futures::StreamExt as _;
    use futures::TryStreamExt as _;

    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_thread_ids(true)
        .with_level(true)
        .init();

    let parallelism = std::thread::available_parallelism()
        .expect("Failed to retrieve available parallelism")
        .get()
        .saturating_sub(MAX_CONCURRENT_REQUESTS.get());
    let parallelism = std::num::NonZeroUsize::new(parallelism)
        .expect("Root computation requires at least 2 threads to run");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(MAX_CONCURRENT_REQUESTS.get())
        .max_blocking_threads(parallelism.get())
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");

    rt.block_on(async move {
        let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
        let bls_key = bls_signatures::PrivateKey::new(secret.to_string().as_bytes());

        let client_eth = eth::Client::new(args.eth_url.as_ref(), None)
            .await
            .expect("Failed to create eth client");
        let client_cc3 = cc_client::Client::new(args.cc3_url, &secret.to_string())
            .await
            .expect("Failed to create cc3 client");

        let config = stream_attestation::ConfigBuilder::new()
            .with_eth(
                stream_eth::roots::ConfigBuilder::new()
                    .with_client(client_eth)
                    .with_start_height(args.start_height)
                    .with_finalization_lag(FINALIZATION_LAG)
                    .with_max_concurrency(MAX_CONCURRENT_REQUESTS)
                    .with_max_parallelism(parallelism)
                    .build(),
            )
            .with_client(client_cc3)
            .with_chain_key(2u64)
            .with_bls_key(bls_key)
            .with_interval_attestation(INTERVAL_ATTESTATION)
            .with_digest_prev(attestor_primitives::Digest::default())
            .with_max_catchup(MAX_CATCHUP)
            .build();

        let mut attestations = stream_attestation::StreamAttestation::new(config)
            .await
            .expect("Failed to create attestation stream");
        let mut n = 0;

        while let Some(permit) = attestations
            .by_ref()
            .try_next()
            .await
            .expect("Failed to fetch permit")
        {
            tracing::info!(?permit, "Generating attestation...");

            n += 1;
            let finalized = INTERVAL_ATTESTATION.get() * n;

            if finalized % MAX_CATCHUP.get() == 0 {
                tracing::warn!(finalized, "New finalized attestation");
                attestations.note_attestation_finalization(finalized);
            }
        }
    })
}
