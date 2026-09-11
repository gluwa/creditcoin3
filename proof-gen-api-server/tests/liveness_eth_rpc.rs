//! Liveness regression tests for the source-chain RPC layer: tip failover, lock-free repair,
//! shared repairs and tolerant fallback startup. Endpoints are wiremock servers on loopback.

use std::sync::Arc;
use std::time::Duration;

use continuity::rpc::{EthRpcProvider, ReconnectingEthRpcProvider};
use serde_json::{json, Value};
use wiremock::{matchers::method, Mock, MockServer, ResponseTemplate};

const ENCODING: usc_abi_encoding::common::EncodingVersion =
    usc_abi_encoding::common::EncodingVersion::V1;

/// JSON-RPC fixture: `eth_chainId` → 31337, `eth_blockNumber` → 1000 (or an error when
/// `fail_tip`), everything under `delay`.
async fn rpc_fixture(server: &MockServer, fail_tip: bool, delay: Duration) {
    Mock::given(method("POST"))
        .respond_with(move |request: &wiremock::Request| {
            let req: Value = request.body_json().unwrap();
            let body = if fail_tip && req["method"] == "eth_blockNumber" {
                json!({"jsonrpc": "2.0", "id": req["id"],
                       "error": {"code": -32000, "message": "upstream unavailable"}})
            } else {
                let result = if req["method"] == "eth_chainId" { "0x7a69" } else { "0x3e8" };
                json!({"jsonrpc": "2.0", "id": req["id"], "result": result})
            };
            ResponseTemplate::new(200)
                .set_body_json(body)
                .set_delay(delay)
        })
        .mount(server)
        .await;
}

async fn count_method(server: &MockServer, name: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.body_json::<Value>().unwrap()["method"] == name)
        .count()
}

#[tokio::test]
async fn tip_read_is_served_by_a_healthy_fallback_when_the_primary_fails() {
    let primary = MockServer::start().await;
    let fallback = MockServer::start().await;
    rpc_fixture(&primary, true, Duration::ZERO).await;
    rpc_fixture(&fallback, false, Duration::ZERO).await;
    let client = eth::Client::new_with_fallbacks(&primary.uri(), &[fallback.uri()], None)
        .await
        .unwrap();
    let provider = ReconnectingEthRpcProvider::new(client, ENCODING);

    assert_eq!(provider.get_last_block().await.unwrap(), 1000);
    assert!(
        count_method(&fallback, "eth_blockNumber").await >= 1,
        "the fallback must have answered the tip read"
    );
    assert_eq!(provider.generation(), 0, "no repair was needed");
}

#[tokio::test]
async fn a_slow_repair_does_not_block_other_callers() {
    let server = MockServer::start().await;
    rpc_fixture(&server, false, Duration::ZERO).await;
    let client = eth::Client::new(&server.uri(), None).await.unwrap();
    let provider = Arc::new(ReconnectingEthRpcProvider::new(client, ENCODING));

    // Fail the operation immediately; the repair then blocks on a slow chain-id read.
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let req: Value = request.body_json().unwrap();
            if req["method"] == "eth_chainId" {
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(10))
                    .set_body_json(json!({"jsonrpc": "2.0", "id": req["id"], "result": "0x7a69"}))
            } else {
                ResponseTemplate::new(503)
            }
        })
        .mount(&server)
        .await;
    let worker_provider = provider.clone();
    let worker = tokio::spawn(async move { worker_provider.get_last_block().await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while count_method(&server, "eth_chainId").await == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the worker must have started its repair dial");

    // Upstream is healthy again for every NEW request while the worker's dial still hangs.
    server.reset().await;
    rpc_fixture(&server, false, Duration::ZERO).await;
    let other = tokio::time::timeout(Duration::from_secs(2), provider.get_last_block())
        .await
        .expect("an unrelated caller must not wait behind another caller's dial");
    assert_eq!(other.unwrap(), 1000);

    worker.abort();
    let _ = worker.await;
}

#[tokio::test]
async fn concurrent_failures_share_one_repair_dial() {
    let server = MockServer::start().await;
    rpc_fixture(&server, false, Duration::ZERO).await;
    let client = eth::Client::new(&server.uri(), None).await.unwrap();
    let provider = Arc::new(ReconnectingEthRpcProvider::new(client, ENCODING));
    assert_eq!(count_method(&server, "eth_chainId").await, 1, "initial connect");

    // Tip reads always fail; each repair dial takes 400 ms.
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let req: Value = request.body_json().unwrap();
            if req["method"] == "eth_chainId" {
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_json(json!({"jsonrpc": "2.0", "id": req["id"], "result": "0x7a69"}))
            } else {
                ResponseTemplate::new(503)
            }
        })
        .mount(&server)
        .await;

    let a = provider.clone();
    let b = provider.clone();
    let (ra, rb) = tokio::join!(a.get_last_block(), b.get_last_block());
    assert!(ra.is_err() && rb.is_err());

    // Two callers × (3 attempts, 2 repairs each) would be 4 dials unshared. Shared: the
    // caller that loses the race sees a newer generation and retries without dialling, so
    // there is exactly one dial per retry round.
    assert_eq!(count_method(&server, "eth_chainId").await, 2);
    assert_eq!(provider.generation(), 2);
}

#[tokio::test]
async fn an_unreachable_fallback_is_skipped_at_startup_but_a_wrong_chain_is_fatal() {
    let primary = MockServer::start().await;
    rpc_fixture(&primary, false, Duration::ZERO).await;

    let dead = "http://127.0.0.1:1".to_string();
    let client = eth::Client::new_with_fallbacks(&primary.uri(), &[dead], None)
        .await
        .expect("a dead backup must not stop a healthy primary");
    assert_eq!(client.get_last_block().await.unwrap(), 1000);

    let wrong_chain = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(|request: &wiremock::Request| {
            let req: Value = request.body_json().unwrap();
            ResponseTemplate::new(200)
                .set_body_json(json!({"jsonrpc": "2.0", "id": req["id"], "result": "0x1"}))
        })
        .mount(&wrong_chain)
        .await;
    let err = eth::Client::new_with_fallbacks(&primary.uri(), &[wrong_chain.uri()], None)
        .await
        .expect_err("a backup on another chain is a misconfiguration");
    assert!(format!("{err:#}").contains("chain_id"), "{err:#}");
}
