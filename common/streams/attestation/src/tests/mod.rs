mod fixtures;
mod macros;

pub(crate) mod mock;

use crate::prelude::*;
use fixtures::*;

/// Polling the attestation stream should yield available attestations.
#[rstest::rstest]
#[tokio::test]
async fn attestation_ready_simple(
    #[future]
    #[with(0, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    tip.send_ready().await;
    roots.send_ready().await;
    poll!(stream_attestation);
}

/// `note_attestation_finalization` should set computed to a valid range. Values like 1..0 are
/// invalid as the end of the range is used is assumed to be strictly greater than its start.
#[rstest::rstest]
#[tokio::test]
async fn attestation_finalize_sets_correct_range(
    #[future]
    #[with(0, nonzero!(1), nonzero!(1))]
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
    #[with(2, nonzero!(1), nonzero!(1))]
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

/// Attestation continuity proof should not include the block before it
#[rstest::rstest]
#[tokio::test]
async fn continuity_proof_size_valid(
    #[future]
    #[with(0, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await;
    roots.send_ready().await;
    tip.send_ready().await;
    tip.send_ready().await;

    poll!(stream_attestation);
}

/// If the attestor lags behind, it is possible to finalize an attestation beyond its local view of
/// the chain. Outdated data returned by the root stream needs to be skipped if this is the case.
#[rstest::rstest]
#[tokio::test]
async fn skip_behind_finality(
    #[future]
    #[with(0, nonzero!(1), nonzero!(2))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready().await; // 0
    roots.send_ready().await; // 1

    assert!(poll!(stream_attestation).is_pending());

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 2,
        ..Default::default()
    });

    tip.send_ready().await; // 0
    tip.send_ready().await; // 1
    tip.send_ready().await; // 2
    tip.send_ready().await; // 3

    roots.send_ready().await; // 2 - behind finality, will be skipped
    roots.send_ready().await; // 3

    let std::task::Poll::Ready(Some(Ok(attestation))) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 3);
    assert!(attestation.continuity_proof.is_empty());
}
