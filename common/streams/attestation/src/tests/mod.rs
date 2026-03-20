mod fixtures;
mod macros;

use crate::prelude::*;
use fixtures::*;

#[rstest::rstest]
#[tokio::test]
async fn attestation_finalize_sets_correct_range(
    #[allow(unused)] logs: (),

    #[future]
    #[with(0, 0, nonzero!(1), nonzero!(1))]
    attestations: (
        futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
        futures::channel::mpsc::UnboundedSender<std::task::Poll<()>>,
        crate::StreamAttestation,
    ),
) {
    use futures::SinkExt as _;

    let (mut permit_roots, mut permit_tip, mut stream_attestation) = attestations.await;

    permit_roots.send(std::task::Poll::Ready(())).await.unwrap();
    permit_roots.send(std::task::Poll::Ready(())).await.unwrap();
    permit_roots.send(std::task::Poll::Ready(())).await.unwrap();
    poll!(stream_attestation);

    stream_attestation.note_attestation_finalization(stream_util::AttestationInfo {
        height: 1,
        ..Default::default()
    });

    permit_tip.send(std::task::Poll::Ready(())).await.unwrap();
    permit_tip.send(std::task::Poll::Ready(())).await.unwrap();
    poll!(stream_attestation);
}
