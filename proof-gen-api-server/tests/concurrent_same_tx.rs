use axum::{body::Body, http::Request};
use futures::future::join_all;
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
mod integration_common;

// Concurrency test: issue multiple simultaneous requests for the same tx proof.
// Expect all to succeed; exactly one uncached, the rest cached (order race tolerant).
#[tokio::test]
async fn concurrent_same_tx_requests() {
    let chain_key = 2u64;
    let header_number = 10u64;
    let tx_index = 0usize;
    let app = integration_common::start_app_with_postgres(chain_key).await;
    let uri = format!("/api/v1/proof/{chain_key}/{header_number}/{tx_index}");

    // Fire 5 concurrent requests
    let mut futs = Vec::new();
    for _ in 0..5 {
        let app_clone = app.clone();
        let uri_clone = uri.clone();
        futs.push(tokio::spawn(async move {
            let req = Request::builder()
                .uri(&uri_clone)
                .method("GET")
                .body(Body::empty())
                .unwrap();
            let resp = app_clone.oneshot(req).await.expect("request failed");
            let status = resp.status().as_u16();
            let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap();
            (status, body)
        }));
    }

    let results = join_all(futs).await;
    let mut success = 0;
    let mut cached_true = 0;
    let mut cached_false = 0;
    for r in results {
        let (status, body) = r.expect("join");
        assert_eq!(status, 200, "all concurrent requests must succeed");
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        if json["cached"].as_bool().unwrap() {
            cached_true += 1;
        } else {
            cached_false += 1;
        }
        success += 1;
    }
    assert_eq!(success, 5, "expected all 5 requests to succeed");
    assert!(
        cached_false >= 1,
        "at least one request should be the non-cached generator"
    );
    if cached_true == 0 {
        // All initial concurrent responses were uncached (expected with async insert). Poll until cached appears.
        let mut cached_seen = false;
        for _ in 0..10 {
            // ~500ms total
            let req = Request::builder()
                .uri(&uri)
                .method("GET")
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.expect("poll request");
            assert_eq!(resp.status().as_u16(), 200);
            let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
            if json["cached"].as_bool().unwrap() {
                cached_seen = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        assert!(
            cached_seen,
            "expected cached response after polling following concurrency burst"
        );
    }
}
