use crate::nonzero;

#[rstest::fixture]
pub fn logs() {
    let _ = tracing_subscriber::fmt()
        .with_level(true)
        .with_test_writer()
        .try_init();
}

#[rstest::fixture]
async fn roots(
    #[default(0)] start_height: attestor_primitives::Height,
) -> (
    futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
    crate::simulation::Roots,
) {
    crate::simulation::Roots::new(start_height)
}

#[rstest::fixture]
async fn tip(
    #[default(0)] start_height: attestor_primitives::Height,
) -> (
    futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
    crate::simulation::Tip,
) {
    crate::simulation::Tip::new(start_height)
}

#[rstest::fixture]
async fn attestations(
    #[default("ws://localhost:9944".parse().unwrap())] cc3_url: url::Url,
    #[default(0)] start_height: attestor_primitives::Height,
    #[default(0)] attestation_prev: attestor_primitives::Height,
    #[default(nonzero!(10))] attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    #[default(nonzero!(50))] max_catchup: std::num::NonZero<attestor_primitives::Height>,

    #[future]
    #[with(start_height)]
    roots: (
        futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
        crate::simulation::Roots,
    ),

    #[future]
    #[with(start_height)]
    tip: (
        futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
        crate::simulation::Tip,
    ),
) -> (
    futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
    futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
    crate::StreamAttestation,
) {
    use futures::StreamExt as _;

    let _ = start_height;

    let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
    let bls_key = bls_signatures::PrivateKey::new(secret.to_string().as_bytes());
    let client_cc3 = cc_client::Client::new(cc3_url, &secret.to_string())
        .await
        .expect("Failed to create cc3 client");

    let (permit_roots, stream_roots) = roots.await;
    let (permit_tip, stream_tip) = tip.await;

    let attestation_prev = stream_util::AttestationInfo {
        height: attestation_prev,
        ..Default::default()
    };

    let config = crate::ConfigBuilder::new()
        .with_cc3(client_cc3)
        .with_chain_key(2u64)
        .with_bls_key(bls_key)
        .with_stream_roots(stream_roots.boxed())
        .with_stream_tip(stream_tip.boxed())
        .with_attestation_interval(attestation_interval)
        .with_attestation_prev(attestation_prev)
        .with_max_catchup(max_catchup)
        .build();
    let stream_attestation = crate::StreamAttestation::new(config);

    (permit_roots, permit_tip, stream_attestation)
}
