use axum::{body::Body, http::Request};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
mod integration_common;

// Verify block-level proof caching (no tx_index) works: first call uncached, second cached.
#[tokio::test]
async fn block_level_caching() {
    // Arrange
    let chain_key = 2u64;
    let header_number = 10u64; // within mock attestation range
    let app = integration_common::start_app_with_postgres(chain_key).await;

    // First request (should not be cached)
    let uri = format!("/api/v1/proof/{chain_key}/{header_number}");
    let req = Request::builder()
        .uri(&uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("initial block proof");
    assert_eq!(resp.status().as_u16(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["cached"].as_bool().unwrap(),
        "first block proof should not be cached"
    );
    assert_eq!(json["chain_key"].as_u64().unwrap(), chain_key);
    assert_eq!(json["header_number"].as_u64().unwrap(), header_number);
    assert!(
        json["tx_index"].is_null(),
        "block-level proof must have null tx_index"
    );

    // Second request: insertion is async, so poll until cached or timeout
    let mut cached_observed = false;
    for _ in 0..10 {
        // up to ~500ms total
        let req2 = Request::builder()
            .uri(&uri)
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp2 = app.clone().oneshot(req2).await.expect("cached block proof");
        assert_eq!(resp2.status().as_u16(), 200);
        let body2 = axum::body::to_bytes(resp2.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        if json2["cached"].as_bool().unwrap() {
            cached_observed = true;
            assert_eq!(json2["header_number"].as_u64().unwrap(), header_number);
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        cached_observed,
        "expected cached block proof within polling window"
    );
}
