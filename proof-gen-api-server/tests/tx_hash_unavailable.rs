use axum::body::to_bytes;
use axum::body::Body;
use axum::http::Request;
use continuity::ContinuityBuilder;
use proof_gen_api_server::db::DbManager;
use proof_gen_api_server::services::mock_providers::make_mock_providers;
use proof_gen_api_server::{build_app, ContinuityService};
use std::sync::Arc;
use tower::ServiceExt; // for `oneshot`

#[tokio::test]
async fn tx_hash_endpoint_reports_unavailable() {
    // Minimal env vars so DbManager::new does not panic; we don't actually connect
    std::env::set_var("POSTGRES_HOST", "localhost");
    std::env::set_var("POSTGRES_PORT", "5432");
    std::env::set_var("POSTGRES_USER", "test");
    std::env::set_var("POSTGRES_PASSWORD", "test");
    std::env::set_var("POSTGRES_DB", "test");
    // Setup mock builder/service
    let (cc_provider, eth_provider) = make_mock_providers(2);
    let builder = ContinuityBuilder::new_with_providers(
        continuity::ContinuityConfig {
            cc3_rpc_url: "ws://localhost:9944".into(),
            eth_rpc_url: "http://localhost:8545".into(),
            chain_key: 2,
        },
        cc_provider,
        eth_provider,
    );
    let db = DbManager::new().expect("db manager");
    let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
    let app = build_app(service);

    let tx_hash = "0xdeadbeef"; // arbitrary
    let uri = format!("/api/v1/proof-by-tx/2/{tx_hash}");
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(
        response.status().as_u16(),
        501,
        "Should return 501 Not Implemented style status"
    );
    let body_bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["code"], "TxHashLookupUnavailable");
}
