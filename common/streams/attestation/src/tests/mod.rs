//! Edge cases which were detected with the help of property testing on the attestation stream.

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

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1

    tip.send_ready(); // 0
    tip.send_ready(); // 1

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 1);
    assert!(attestation.continuity_proof.is_empty());
}

/// `note_attestation_finalization` should set `computed` to a valid range. Values like 1..0 are
/// invalid as the end of the range is used is assumed to be strictly greater than its start.
#[rstest::rstest]
#[tokio::test]
async fn attestation_finalization_sets_correct_range(
    #[future]
    #[with(0, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1
    roots.send_ready(); // 2

    assert!(poll!(stream_attestation).is_pending());

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    assert_eq!(stream_attestation.computed, 1..=1); // not 1..=2!

    tip.send_ready(); // 0
    tip.send_ready(); // 1
    tip.send_ready(); // 2

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert!(attestation.continuity_proof.is_empty());

    assert_eq!(stream_attestation.computed, 1..=2);
}

#[rstest::rstest]
#[tokio::test]
async fn attestation_finalization_ignore_past_attestation(
    #[future]
    #[with(0, nonzero!(1), nonzero!(2))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    tip.send_ready(); // 0
    tip.send_ready(); // 1
    tip.send_ready(); // 2

    assert!(poll!(stream_attestation).is_pending());

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1
    roots.send_ready(); // 2

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert_eq!(attestation.continuity_proof.len(), 1);

    // Marks latest root height as finalized
    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 2,
        ..Default::default()
    });

    assert_eq!(stream_attestation.computed, 2..=2);

    // Past finalizations should not update the state of the stream
    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    assert_eq!(stream_attestation.computed, 2..=2);

    tip.send_ready(); // 3
    roots.send_ready(); // 3

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 3);
    assert!(attestation.continuity_proof.is_empty());
}

/// Attestation stream cache size should grow up to `max_catchup` + 1 in order to allow the last
/// block in the cache to be a multiple of the `max_catchup`.
#[rstest::rstest]
#[tokio::test]
async fn max_cache_size(
    #[future]
    #[with(2, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, _tip, mut stream_attestation) = attestations.await;

    roots.send_ready(); // 2 - skipped, start height is always ignored
    roots.send_ready(); // 3
    roots.send_ready(); // 4 - not polled, max cache size reached

    assert!(poll!(stream_attestation).is_pending());

    assert_eq!(
        stream_attestation.cache,
        vec![
            stream_util::RootInfo {
                height: 3,
                ..Default::default()
            },
            stream_util::RootInfo {
                height: 4,
                ..Default::default()
            }
        ]
    )
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

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1

    assert!(poll!(stream_attestation).is_pending());

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 2,
        ..Default::default()
    });

    tip.send_ready(); // 0
    tip.send_ready(); // 1
    tip.send_ready(); // 2
    tip.send_ready(); // 3

    roots.send_ready(); // 2 - skipped, behind finality
    roots.send_ready(); // 3

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 3);
    assert!(attestation.continuity_proof.is_empty());
}

/// If several attestations are produced ahead of finality, the size of their continuity proof
/// should keep growing to encompass new blocks.
#[rstest::rstest]
#[tokio::test]
async fn continuity_proofs_should_grow(
    #[future]
    #[with(0, nonzero!(1), nonzero!(1))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1

    tip.send_ready(); // 0
    tip.send_ready(); // 1

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 1);
    assert!(attestation.continuity_proof.is_empty());

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    roots.send_ready(); // 2
    tip.send_ready(); // 2

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert!(attestation.continuity_proof.is_empty());

    tip.send_ready(); // 3

    assert!(poll!(stream_attestation).is_pending());

    roots.send_ready(); // 3

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 3);
    assert_eq!(attestation.continuity_proof.len(), 1);
}

// If a past attestation finalizes, future attestations have to be regenerated as the prev digest
// has changed.
#[rstest::rstest]
#[tokio::test]
async fn regenerate_attestations(
    #[future]
    #[with(0, nonzero!(1), nonzero!(2))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1
    tip.send_ready(); // 0
    tip.send_ready(); // 1

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 1);
    assert!(attestation.continuity_proof.is_empty());

    tip.send_ready(); // 2
    roots.send_ready(); // 2

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert_eq!(attestation.continuity_proof.len(), 1);

    tip.send_ready(); // 3
    roots.send_ready(); // 3

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 3);
    assert_eq!(attestation.continuity_proof.len(), 2);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    // Attestation 3 is re-generated. Notice that the continuity proof is shorter, as it now
    // attests from block 1 instead of block 0.
    assert_eq!(attestation.header_number(), 3);
    assert_eq!(attestation.continuity_proof.len(), 1);

    // Attestation 2 is regenerated as well
    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert!(attestation.continuity_proof.is_empty());

    // Attestation 1 is not regenerated as it has already finalized
    assert!(poll!(stream_attestation).is_pending());
}

#[rstest::rstest]
#[tokio::test]
async fn attestation_chain_reversion(
    #[future]
    #[with(0, nonzero!(1), nonzero!(2))]
    attestations: (mock::RootSender, mock::TipSender, crate::StreamAttestation),
) {
    let (mut roots, mut tip, mut stream_attestation) = attestations.await;

    roots.send_ready(); // 0 - skipped, start height is always ignored
    roots.send_ready(); // 1
    roots.send_ready(); // 2

    tip.send_ready(); // 0
    tip.send_ready(); // 1
    tip.send_ready(); // 2

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 1);
    assert!(attestation.continuity_proof.is_empty());

    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    assert_eq!(attestation.header_number(), 2);
    assert_eq!(attestation.continuity_proof.len(), 1);

    // Reset the chain at height 1
    stream_attestation
        .note_attestation_chain_reversion(stream_util::AttestationInfo {
            height: 1,
            ..Default::default()
        })
        .await;

    roots.send_ready(); // 1 - skipped, start height is always ignored
    roots.send_ready(); // 2

    tip.send_ready(); // 1
    tip.send_ready(); // 2

    // Attestation at height 1 is re-generated since the chain has been reverted to height 0
    let std::task::Poll::Ready(Some(attestation)) = poll!(stream_attestation) else {
        panic!("Failed to generate attestation");
    };

    // Continuity proof is now empty as we are attesting from height 1
    assert_eq!(attestation.header_number(), 2);
    assert!(attestation.continuity_proof.is_empty());
}
