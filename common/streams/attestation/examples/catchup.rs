#[derive(clap::Parser)]
struct Args {
    #[arg(long, default_value_t = url::Url::parse("ws://localhost:8545").unwrap())]
    eth_url: url::Url,

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
        let client = eth::Client::new(args.eth_url.as_ref(), None)
            .await
            .expect("Failed to create eth client");

        let config = stream_attestation::ConfigBuilder::new()
            .with_eth(
                stream_eth::roots::ConfigBuilder::new()
                    .with_client(client)
                    .with_start_height(args.start_height)
                    .with_finalization_lag(FINALIZATION_LAG)
                    .with_max_concurrency(MAX_CONCURRENT_REQUESTS)
                    .with_max_parallelism(parallelism)
                    .build(),
            )
            .with_interval_attestation(INTERVAL_ATTESTATION)
            .with_max_catchup(MAX_CATCHUP)
            .build();

        let mut attestations = stream_attestation::StreamAttestation::new(config)
            .await
            .expect("Failed to create attestation stream");

        while let Some(permit) = attestations
            .by_ref()
            .try_next()
            .await
            .expect("Failed to fetch permit")
        {
            tracing::info!(?permit, "Generating attestation...");

            let height = permit.height();
            if height % MAX_CATCHUP.get() == INTERVAL_ATTESTATION.get() {
                let finalized = MAX_CATCHUP.get() * (height / MAX_CATCHUP.get() + 1);

                tracing::warn!(finalized, "New finalized attestation");
                attestations.note_attestation_finalization(finalized);
            }
        }
    })
}
