use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use continuity::{ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::{
    networking::build_app,
    services::{continuity_service::ContinuityService, mock_providers::make_mock_providers},
};
use std::sync::Arc;
use tower::util::ServiceExt; // for oneshot helper

#[tokio::test]
async fn continuity_endpoint_returns_proof() {
    // Arrange: mock providers & builder
    let chain_key = 2u64;
    let header_number = 10u64; // falls between mock attestations 5 and 15
    let (cc_provider, eth_provider) = make_mock_providers(chain_key);
    let config = ContinuityConfig {
        cc3_rpc_url: "ws://mock".into(),
        eth_rpc_url: "ws://mock".into(),
        chain_key,
    };
    let builder = ContinuityBuilder::new_with_providers(config, cc_provider, eth_provider);
    // Provide dummy Postgres env vars so DbManager::new() succeeds; test focuses on HTTP + serialization, not DB IO.
    std::env::set_var("POSTGRES_HOST", "localhost");
    std::env::set_var("POSTGRES_PORT", "5432");
    std::env::set_var("POSTGRES_USER", "test");
    std::env::set_var("POSTGRES_PASSWORD", "test");
    std::env::set_var("POSTGRES_DB", "test");
    let db = proof_gen_api_server::db::DbManager::new().expect("DB manager init");
    let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
    let app = build_app(service);

    let uri = format!("/api/v1/proof/{chain_key}/{header_number}");
    let request = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();

    // Act
    let response = app.clone().oneshot(request).await.unwrap();

    // Assert
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["chain_key"].as_u64().unwrap(), chain_key);
    assert_eq!(json["header_number"].as_u64().unwrap(), header_number);
    assert!(!json["continuity_proof"]["blocks"]
        .as_array()
        .unwrap()
        .is_empty());
}
