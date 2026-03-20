mod fixtures;
mod macros;

pub(crate) mod mock;

use crate::prelude::*;
use fixtures::*;

/// `note_attestation_finalization` should set computed to a valid range. Values like 1..0 are
/// invalid as the end of the range is used is assumed to be strictly greater than its start.
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

/// Attestation stream cache size should be equal to `max_catchup` + 1 in order to allow the last
/// block in the cache to be a multiple of the `max_catchup`.
#[rstest::rstest]
#[tokio::test]
async fn cache_size(
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
