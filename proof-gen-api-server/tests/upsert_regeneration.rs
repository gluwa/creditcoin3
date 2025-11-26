use axum::{body::Body, http::Request};
use tokio::time::{sleep, Duration};
use tower::ServiceExt;
mod integration_common;

// Upsert semantics: regenerate same tx proof and ensure cached flag true and structure consistent.
#[tokio::test]
async fn upsert_regeneration_updates_entry() {
    let chain_key = 2u64;
    let header_number = 10u64;
    let tx_index = 0usize;
    let app = integration_common::start_app_with_postgres(chain_key).await;
    let uri = format!("/api/v1/proof/{chain_key}/{header_number}/{tx_index}");

    // Initial generation
    let req1 = Request::builder()
        .uri(&uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.expect("first proof");
    assert_eq!(resp1.status().as_u16(), 200);
    let body1 = axum::body::to_bytes(resp1.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
    assert!(
        !json1["cached"].as_bool().unwrap(),
        "first proof should not be cached"
    );
    let tx_hash = json1["tx_hash"].as_str().unwrap().to_string();

    // Poll the tx-hash endpoint until the cached row is visible (background insert completes)
    let tx_hash_uri = format!("/api/v1/proof-by-tx/{chain_key}/{tx_hash}");
    let mut cached_row: Option<serde_json::Value> = None;
    for _ in 0..40 {
        // up to ~2s
        let req_h = Request::builder()
            .uri(&tx_hash_uri)
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp_h = app.clone().oneshot(req_h).await.expect("tx-hash lookup");
        if resp_h.status().as_u16() == 200 {
            let body_h = axum::body::to_bytes(resp_h.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let json_h: serde_json::Value = serde_json::from_slice(&body_h).unwrap();
            if json_h["cached"].as_bool().unwrap() {
                cached_row = Some(json_h);
                break;
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    let cached_json =
        cached_row.expect("expected cached row via tx-hash endpoint within polling window");
    assert_eq!(cached_json["tx_hash"].as_str().unwrap(), tx_hash);

    // Now call the original tx-index endpoint again; should be cached immediately
    let req_final = Request::builder()
        .uri(&uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp_final = app
        .clone()
        .oneshot(req_final)
        .await
        .expect("final regeneration call");
    assert_eq!(resp_final.status().as_u16(), 200);
    let body_final = axum::body::to_bytes(resp_final.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json_final: serde_json::Value = serde_json::from_slice(&body_final).unwrap();
    assert!(
        json_final["cached"].as_bool().unwrap(),
        "final regeneration should be cached"
    );
    assert_eq!(json_final["tx_hash"].as_str().unwrap(), tx_hash);
}
