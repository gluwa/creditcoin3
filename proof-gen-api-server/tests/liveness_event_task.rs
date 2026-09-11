//! Liveness regression tests for the cc3 event task and the shared Creditcoin client.
//! All endpoints are disposable loopback fixtures; no live chain is touched.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use cc_client::ws_fixture::WsFixture;

/// The server hands one `Arc<CcClient>` to the builders and the event task. A reconnect
/// through any holder must repair every holder — that is the property the `Arc` buys and the
/// reason value-cloning (`CcClient::clone`, a fresh connection slot) is no longer used.
#[tokio::test]
async fn reconnect_through_the_shared_arc_repairs_every_holder() {
    let fixture = WsFixture::start().await;
    let builder_client = Arc::new(
        cc_client::Client::new_read_only(&fixture.url)
            .await
            .unwrap(),
    );
    let event_client = builder_client.clone();
    assert!(builder_client
        .legacy()
        .chain_get_finalized_head()
        .await
        .is_ok());

    fixture.close_open_connections();
    tokio::time::timeout(Duration::from_secs(2), async {
        while builder_client
            .legacy()
            .chain_get_finalized_head()
            .await
            .is_ok()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("client must notice the closed socket");

    event_client.reconnect().await.unwrap();
    assert!(event_client
        .legacy()
        .chain_get_finalized_head()
        .await
        .is_ok());
    assert!(
        builder_client
            .legacy()
            .chain_get_finalized_head()
            .await
            .is_ok(),
        "the builders' handle must observe the event task's reconnect"
    );
    assert_eq!(
        fixture.accepted.load(Ordering::SeqCst),
        2,
        "exactly one redial"
    );
}

/// Permanently pruned history ends the stream; the event task must return an error (the
/// supervisor in `Server::run` turns that into a nonzero process exit) and must not keep
/// looping on a dead subscription.
#[tokio::test]
async fn pruned_history_makes_the_event_task_return_an_error() {
    let fixture = WsFixture::start().await;
    // Heads 1 and 2 are pushed on subscribe; the second event read hits pruned state.
    fixture.chain.prune_events_after(1);
    fixture.finalize(1);
    fixture.finalize(2);
    let cc = Arc::new(
        cc_client::Client::new_read_only(&fixture.url)
            .await
            .unwrap(),
    );
    let (cc_provider, eth_provider) = continuity::mocks::make_mock_providers(2);
    let cfg = continuity::ContinuityConfig::builder()
        .cc3_rpc_url(&fixture.url)
        .eth_rpc_url("http://mock")
        .chain_key(2)
        .attestation_interval(10)
        .checkpoint_interval(10)
        .build();
    let builder = Arc::new(continuity::ContinuityBuilder::new_with_providers(
        cfg,
        cc_provider,
        eth_provider,
    ));
    let metrics = Arc::new(proof_gen_api_server::prom::ProofGenMetrics::new(&[2]));
    let service = Arc::new(
        proof_gen_api_server::ContinuityService::new(vec![builder], metrics.clone(), 10, 1000)
            .await
            .unwrap(),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        proof_gen_api_server::events::start_cc3_event_subscription(
            cc,
            Arc::default(),
            Arc::default(),
            service.clone(),
            None,
        ),
    )
    .await
    .expect("event task must exit on pruned history");
    let err = result.expect_err("end of stream is an error, not a clean return");
    assert!(
        err.to_string().contains("End of unbounded event stream"),
        "{err}"
    );

    // What the supervisor does next: the replica reports itself dead and degraded.
    service.mark_event_stream_dead(&err.to_string());
    assert!(service.event_stream_dead().is_some());
}
