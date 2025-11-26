use axum::{body::Body, http::Request};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
mod integration_common;

// Ensure multiple tx proofs within same block cache independently.
#[tokio::test]
async fn multi_tx_independent_caching() {
    let chain_key = 2u64;
    let header_number = 10u64;
    let app = integration_common::start_app_with_postgres(chain_key).await;

    // First TX index 0
    let uri0 = format!("/api/v1/proof/{chain_key}/{header_number}/0");
    let req0 = Request::builder()
        .uri(&uri0)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp0 = app.clone().oneshot(req0).await.expect("tx0 first");
    assert_eq!(resp0.status().as_u16(), 200);
    let bytes0 = axum::body::to_bytes(resp0.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json0: serde_json::Value = serde_json::from_slice(&bytes0).unwrap();
    assert!(
        !json0["cached"].as_bool().unwrap(),
        "tx0 first call not cached"
    );

    // First TX index 1
    let uri1 = format!("/api/v1/proof/{chain_key}/{header_number}/1");
    let req1 = Request::builder()
        .uri(&uri1)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.expect("tx1 first");
    assert_eq!(resp1.status().as_u16(), 200);
    let bytes1 = axum::body::to_bytes(resp1.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json1: serde_json::Value = serde_json::from_slice(&bytes1).unwrap();
    assert!(
        !json1["cached"].as_bool().unwrap(),
        "tx1 first call not cached"
    );

    // Second calls should be cached independently (poll due to async insert)
    for (uri, label) in [(&uri0, "tx0"), (&uri1, "tx1")] {
        let mut cached_seen = false;
        for _ in 0..10 {
            // up to ~500ms
            let req = Request::builder()
                .uri(uri)
                .method("GET")
                .body(Body::empty())
                .unwrap();
            let resp = app.clone().oneshot(req).await.expect("second call");
            assert_eq!(resp.status().as_u16(), 200);
            let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if json["cached"].as_bool().unwrap() {
                cached_seen = true;
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
        assert!(
            cached_seen,
            "expected cached second call for {label} within polling window"
        );
    }
}
