#[derive(clap::Parser)]
struct Args {
    #[arg(long, default_value_t = url::Url::parse("ws://localhost:8545").unwrap())]
    eth_url: url::Url,

    #[arg(long, default_value_t = 0)]
    start_height: attestor_primitives::Height,

    #[arg(long, default_value_t = std::num::NonZero::new(10).unwrap())]
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,

    #[arg(long, default_value_t = std::num::NonZeroUsize::new(1000).unwrap())]
    blocks: std::num::NonZeroUsize,
}

const FINALIZATION_LAG: attestor_primitives::Height = 10;
const MAX_CONCURRENT_REQUESTS: std::num::NonZeroUsize = std::num::NonZeroUsize::new(10).unwrap();
const MAX_CATCHUP: std::num::NonZeroU64 = std::num::NonZeroU64::new(50).unwrap();

fn main() {
    use clap::Parser as _;
    use futures::StreamExt as _;
    use stream_util::ChainExt as _;

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
        .expect("Root computation requires more than MAX_CONCURRENT_REQUESTS threads to run");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(MAX_CONCURRENT_REQUESTS.get() + 1)
        .max_blocking_threads(parallelism.get())
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");

    rt.block_on(async move {
        let client_eth = eth::Client::new(args.eth_url.as_ref(), None)
            .await
            .expect("Failed to create eth client");

        let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
        let attestor = cc_client::attestor::Attestor::new(secret.into(), 2)
            .expect("Failed to initialize attestor");

        let config = stream_eth::roots::ConfigBuilder::new()
            .with_client(client_eth.clone())
            .with_start_height(args.start_height)
            .with_finalization_lag(FINALIZATION_LAG)
            .with_max_concurrency(MAX_CONCURRENT_REQUESTS)
            .with_max_parallelism(parallelism)
            .build();
        let stream_roots = stream_eth::StreamRoots::new(config).await.boxed_data();

        let config = stream_eth::tip::ConfigBuilder::new()
            .with_client(client_eth.clone())
            .with_finalization_lag(FINALIZATION_LAG)
            .with_start_height(args.start_height)
            .build();
        let stream_tip = stream_eth::StreamTip::new(config).await.boxed_data();

        let config = stream_attestation::ConfigBuilder::new()
            .with_chain_key(2u64)
            .with_stream_roots(stream_roots)
            .with_stream_tip(stream_tip)
            .with_attestation_interval(args.attestation_interval)
            .with_attestation_prev(stream_util::AttestationInfo::default())
            .with_max_catchup(MAX_CATCHUP)
            .build();

        let mut attestations = stream_attestation::StreamAttestation::new(config);

        tracing::info!(height = 0, "Generating genesis attestation...");

        let block = client_eth
            .get_block(
                args.start_height,
                usc_abi_encoding::common::EncodingVersion::V1,
            )
            .await
            .expect("Failed to retrieve genesis block");

        let height = args.start_height;
        let root = eth::simple_merkle_tree(&block).root();
        let hash = attestor_primitives::Digest::from(*block.hash());

        let genesis = attestations
            .generate_attestation_genesis(&attestor, stream_util::RootInfo { height, root, hash });
        let digest = genesis.digest();
        let info = stream_util::AttestationInfo { digest, height };

        tracing::info!(%digest, "New genesis attestation");

        attestations.note_attestation_finalization(info);

        let mut n = 0;
        while let Some(permit) = attestations.by_ref().next().await {
            let attestation = attestations.generate_attestation(&attestor, permit);

            let digest = attestation.digest();
            let height = attestation.header_number();

            tracing::info!(height, %digest, "New attestation");

            n += 1;
            let finalized = args.attestation_interval.get() * n;

            if finalized % attestations.max_catchup() == 0 {
                tracing::warn!(finalized, "New finalized attestation");

                let info = stream_util::AttestationInfo {
                    digest,
                    height: finalized,
                };

                attestations.note_attestation_finalization(info);
            }
        }
    })
}
