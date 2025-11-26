use axum::{body::Body, http::Request};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
mod integration_common;

// Reset DB flow: after generating a proof, simulate a reset by dropping & recreating schema via DbManager.reset_db.
#[tokio::test]
async fn reset_db_clears_cache_state() {
    let chain_key = 2u64;
    let header_number = 10u64;
    let tx_index = 0usize;
    let app = integration_common::start_app_with_postgres(chain_key).await;

    // First proof generation (uncached)
    let uri = format!("/api/v1/proof/{chain_key}/{header_number}/{tx_index}");
    let req = Request::builder()
        .uri(&uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.expect("initial proof");
    assert_eq!(resp.status().as_u16(), 200);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["cached"].as_bool().unwrap(),
        "first generation should not be cached"
    );

    // Second call: should be cached (poll due to async insert)
    let mut cached_seen = false;
    for _ in 0..10 {
        // ~500ms window
        let req2 = Request::builder()
            .uri(&uri)
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp2 = app.clone().oneshot(req2).await.expect("second proof");
        let body2 = axum::body::to_bytes(resp2.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let json2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        if json2["cached"].as_bool().unwrap() {
            cached_seen = true;
            break;
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        cached_seen,
        "second generation should be cached within polling window"
    );

    // Perform DB reset using a fresh DbManager (env vars already set by start helper)
    let db = proof_gen_api_server::db::DbManager::new().expect("db manager init");
    db.reset_db().await.expect("reset db");

    // Third call after reset should regenerate (cached=false again)
    let req3 = Request::builder()
        .uri(&uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp3 = app
        .clone()
        .oneshot(req3)
        .await
        .expect("third proof post-reset");
    let body3 = axum::body::to_bytes(resp3.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json3: serde_json::Value = serde_json::from_slice(&body3).unwrap();
    assert!(
        !json3["cached"].as_bool().unwrap(),
        "after reset, proof should regenerate uncached"
    );
}
