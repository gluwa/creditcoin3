use axum::{body::Body, http::Request};
use proof_gen_api_server::{build_app, ContinuityService};
use serde_json::Value;
use std::sync::Arc;
use tower::util::ServiceExt;

// NOTE: This test is illustrative; actual triggering of specific errors may require
// crafting inputs. Here we force a TxIndexOutOfBounds by using a very large tx_index.

#[tokio::test]
async fn tx_index_out_of_bounds_maps_to_bad_request() {
    // Build app with mock providers through existing helper
    let chain_key = 2u64;
    let (cc, eth) = proof_gen_api_server::mock_providers::make_mock_providers(chain_key);
    let builder = continuity::builder::ContinuityBuilder::new_with_providers(
        continuity::config::ContinuityConfig {
            chain_key,
            cc3_rpc_url: "http://unused".into(),
            eth_rpc_url: "http://unused".into(),
        },
        cc,
        eth,
    );
    // Provide required env vars for DbManager::new (will fail connection later silently when used in background).
    std::env::set_var("POSTGRES_HOST", "localhost");
    std::env::set_var("POSTGRES_PORT", "5432");
    std::env::set_var("POSTGRES_USER", "postgres");
    std::env::set_var("POSTGRES_PASSWORD", "postgres");
    std::env::set_var("POSTGRES_DB", "postgres");
    let db = Arc::new(proof_gen_api_server::db::DbManager::new().expect("db manager init"));
    let service = Arc::new(ContinuityService::new(Arc::new(builder), db));
    let app = build_app(service);

    // Use a very large tx_index to trigger out of bounds
    let request = Request::builder()
        .uri("/api/v1/proof/2/10/9999")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "TxIndexOutOfBounds");
    assert!(body["message"].as_str().unwrap().contains("out of bounds"));
}
