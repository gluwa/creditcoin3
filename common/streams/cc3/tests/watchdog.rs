//! Progress-watchdog tests for `StreamCC3` against an in-process fake Creditcoin node.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use cc_client::ws_fixture::WsFixture;
use futures::StreamExt;

async fn stream_with(
    fixture: &WsFixture,
    progress_timeout: Duration,
) -> (stream_cc3::StreamCC3, Arc<stream_cc3::Progress>) {
    let cc3 = Arc::new(
        cc_client::Client::new_read_only(&fixture.url)
            .await
            .unwrap(),
    );
    let progress = Arc::new(stream_cc3::Progress::default());
    let config = stream_cc3::ConfigBuilder::new()
        .with_cc3(cc3)
        .with_chain_keys(vec![1])
        .with_progress_timeout(progress_timeout)
        .with_progress(Some(progress.clone()))
        .build();
    let stream = tokio::time::timeout(Duration::from_secs(10), stream_cc3::StreamCC3::new(config))
        .await
        .expect("stream construction must not hang")
        .unwrap();
    (stream, progress)
}

async fn next_height(stream: &mut stream_cc3::StreamCC3) -> u64 {
    tokio::time::timeout(Duration::from_secs(10), stream.next())
        .await
        .expect("stream must make progress")
        .expect("stream must not end")
        .block_number()
}

/// The socket stays open and answers RPC, but the subscription stops delivering while the node
/// keeps finalizing. The watchdog must notice, redial once, and the stream must carry on from
/// where it left off with no gap.
#[tokio::test]
async fn silent_subscription_is_replaced_and_the_gap_is_filled() {
    let fixture = WsFixture::start().await;
    fixture.finalize(1);
    let (mut stream, progress) = stream_with(&fixture, Duration::from_millis(400)).await;
    assert_eq!(next_height(&mut stream).await, 1);
    assert_eq!(progress.height(), Some(1));

    fixture.silence_open_subscriptions();
    fixture.finalize(2);
    fixture.finalize(3);

    assert_eq!(next_height(&mut stream).await, 2);
    assert_eq!(next_height(&mut stream).await, 3);
    assert_eq!(progress.height(), Some(3));
    assert_eq!(progress.silent_recoveries(), 1);
    assert_eq!(
        fixture.accepted.load(Ordering::SeqCst),
        2,
        "exactly one redial for one silent subscription"
    );
}

/// No new finalized block anywhere: the chain is stalled, not the subscription. The watchdog
/// must keep waiting on the same connection instead of churning reconnects.
#[tokio::test]
async fn stalled_finality_does_not_reconnect() {
    let fixture = WsFixture::start().await;
    fixture.finalize(1);
    let (mut stream, progress) = stream_with(&fixture, Duration::from_millis(200)).await;
    assert_eq!(next_height(&mut stream).await, 1);

    // Several watchdog periods with nothing to see.
    let idle = tokio::time::timeout(Duration::from_millis(900), stream.next()).await;
    assert!(
        idle.is_err(),
        "nothing should be yielded while finality is stalled"
    );
    assert_eq!(fixture.accepted.load(Ordering::SeqCst), 1, "no redial");
    assert_eq!(progress.silent_recoveries(), 0);
    assert_eq!(progress.height(), Some(1));

    // Finality resumes on the same subscription.
    fixture.finalize(2);
    assert_eq!(next_height(&mut stream).await, 2);
}

/// A subscription that is accepted but never delivers its first block must not hang
/// construction forever; the seed loop reconnects and retries under the same deadline.
#[tokio::test]
async fn seed_does_not_hang_on_a_subscription_that_never_delivers() {
    let fixture = WsFixture::start().await;
    // Finalized head is 0: nothing is pushed on subscribe. Construction spins in the seed
    // retry loop; unblock it after two deadlines by finalizing a block on the (new) socket.
    let cc3 = Arc::new(
        cc_client::Client::new_read_only(&fixture.url)
            .await
            .unwrap(),
    );
    let config = stream_cc3::ConfigBuilder::new()
        .with_cc3(cc3)
        .with_chain_keys(vec![1])
        .with_progress_timeout(Duration::from_millis(300))
        .build();
    let construct = tokio::spawn(stream_cc3::StreamCC3::new(config));
    tokio::time::sleep(Duration::from_millis(750)).await;
    assert!(
        fixture.accepted.load(Ordering::SeqCst) >= 2,
        "seed must have redialed"
    );
    fixture.finalize(1);
    let mut stream = tokio::time::timeout(Duration::from_secs(10), construct)
        .await
        .expect("construction must complete once a block is finalized")
        .unwrap()
        .unwrap();
    assert_eq!(next_height(&mut stream).await, 1);
}
