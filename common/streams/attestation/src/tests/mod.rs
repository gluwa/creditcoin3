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
    let (mut permit_roots, mut permit_tip, mut stream_attestation) = attestations.await;

    permit_roots.send_ready().await;
    permit_roots.send_ready().await;
    permit_roots.send_ready().await;
    poll!(stream_attestation);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    permit_tip.send_ready().await;
    permit_tip.send_ready().await;
    poll!(stream_attestation);
}
