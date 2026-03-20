mod fixtures;
mod macros;

pub(crate) mod mock;

use crate::prelude::*;
use fixtures::*;

#[rstest::rstest]
#[tokio::test]
async fn attestation_finalize_sets_correct_range(
    #[future]
    #[with(0, 0, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await;
    roots.send_ready().await;
    roots.send_ready().await;
    poll!(stream_attestation);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    tip.send_ready().await;
    tip.send_ready().await;
    poll!(stream_attestation);
}

#[rstest::rstest]
#[tokio::test]
async fn simulation_failure(
    #[future]
    #[with(2, 0, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await;
    roots.send_ready().await;
    poll!(stream_attestation);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 2,
        ..Default::default()
    });

    roots.send_ready().await;
    tip.send_ready().await;
    tip.send_ready().await;
    poll!(stream_attestation);
}
