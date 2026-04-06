use crate::nonzero;

#[rstest::fixture]
pub fn attestor(
    #[default([0; 32])] secret: [u8; 32],
    #[default(2)] chain_key: attestor_primitives::ChainKey,
) -> cc_client::attestor::Attestor {
    cc_client::attestor::Attestor::new(secret.into(), chain_key).unwrap()
}

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
    use stream_util::ChainExt as _;

    let (permit_roots, stream_roots) = roots.await;
    let (permit_tip, stream_tip) = tip.await;

    let attestation_prev = stream_util::AttestationInfo {
        height: start_height,
        ..Default::default()
    };

    let config = crate::ConfigBuilder::new()
        .with_chain_key(2u64)
        .with_stream_roots(stream_roots.boxed_data())
        .with_stream_tip(stream_tip.boxed_data())
        .with_attestation_interval(attestation_interval)
        .with_attestation_prev(attestation_prev)
        .with_max_catchup(max_catchup)
        .build();
    let stream_attestation = crate::StreamAttestation::new(config);

    (permit_roots, permit_tip, stream_attestation)
}
