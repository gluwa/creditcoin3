use crate::nonzero;

#[rstest::fixture]
pub async fn roots(
    #[default(0)] start_height: attestor_primitives::Height,
) -> (super::mock::RootSender, super::mock::RootReceiver) {
    super::mock::roots(start_height)
}

#[rstest::fixture]
pub async fn tip(
    #[default(0)] start_height: attestor_primitives::Height,
) -> (super::mock::TipSender, super::mock::TipReceiver) {
    super::mock::tip(start_height)
}

#[rstest::fixture]
pub async fn attestations(
    #[default(0)] start_height: attestor_primitives::Height,
    #[default(nonzero!(10))] attestation_interval: std::num::NonZero<attestor_primitives::Height>,
    #[default(nonzero!(50))] max_catchup: std::num::NonZero<attestor_primitives::Height>,

    #[default("ws://localhost:9944".parse().unwrap())] cc3_url: url::Url,

    #[future]
    #[with(start_height)]
    roots: (super::mock::RootSender, super::mock::RootReceiver),

    #[future]
    #[with(start_height)]
    tip: (super::mock::TipSender, super::mock::TipReceiver),
) -> (
    super::mock::RootSender,
    super::mock::TipSender,
    crate::StreamAttestation,
) {
    use futures::StreamExt as _;

    let secret = bip39::Mnemonic::generate(12).expect("Failed to generate attestor secret");
    let bls_key = bls_signatures::PrivateKey::new(secret.to_string().as_bytes());
    let client_cc3 = cc_client::Client::new(cc3_url, &secret.to_string())
        .await
        .expect("Failed to create cc3 client");

    let (permit_roots, stream_roots) = roots.await;
    let (permit_tip, stream_tip) = tip.await;

    let attestation_prev = stream_util::AttestationInfo {
        height: start_height.saturating_sub(attestation_interval.get()),
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
