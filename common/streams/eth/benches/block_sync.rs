#[derive(clap::Parser)]
struct Args {
    #[arg(long, default_value_t = url::Url::parse("ws://localhost:8545").unwrap())]
    eth_url: url::Url,

    #[arg(long, default_value_t = 0)]
    start_height: attestor_primitives::Height,

    #[arg(long, default_value_t = std::num::NonZeroUsize::new(1000).unwrap())]
    blocks: std::num::NonZeroUsize,

    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    _extra: Vec<String>,
}

const FINALIZATION_LAG: attestor_primitives::Height = 10;
const MAX_CONCURRENT_REQUESTS: std::num::NonZeroUsize = std::num::NonZeroUsize::new(10).unwrap();

fn main() {
    use clap::Parser as _;
    use futures::StreamExt as _;

    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_thread_ids(true)
        .with_level(true)
        .init();

    let parallelism = std::thread::available_parallelism()
        .expect("Failed to retrieve available parallelism")
        .get()
        .saturating_sub(MAX_CONCURRENT_REQUESTS.get() + 1);
    let parallelism = std::num::NonZeroUsize::new(parallelism)
        .expect("Root computation requires at least 2 threads to run");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(MAX_CONCURRENT_REQUESTS.get() + 1)
        .max_blocking_threads(parallelism.get())
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");

    rt.block_on(async move {
        let client = eth::Client::new(args.eth_url.as_ref(), None)
            .await
            .expect("Failed to create eth client");

        let config = stream_eth::roots::ConfigBuilder::new()
            .with_client(client)
            .with_start_height(args.start_height)
            .with_finalization_lag(FINALIZATION_LAG)
            .with_max_concurrency(MAX_CONCURRENT_REQUESTS)
            .with_max_parallelism(parallelism)
            .build();
        let stream_roots = stream_eth::StreamRoots::new(config)
            .await
            .expect("Failed to create roots stream")
            .take(args.blocks.get());
        let mut stream_roots = std::pin::pin!(stream_roots);

        tracing::warn!("Starting benchmark...");

        let now = std::time::Instant::now();

        while let Some(_root) = stream_roots.next().await {}

        let elapsed = now.elapsed().as_millis();

        tracing::warn!(elapsed, "Benchmark complete!");
    })
}
