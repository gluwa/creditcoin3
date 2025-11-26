use axum::{
    body::Body,
    http::{Request, StatusCode},
};
mod integration_common;
use integration_common::start_app_with_postgres;
use tower::util::ServiceExt; // for oneshot helper

#[tokio::test]
async fn continuity_endpoint_returns_proof() {
    // Arrange: app backed by a real Postgres container
    let chain_key = 2u64;
    let header_number = 10u64; // falls between mock attestations 5 and 15
    let app = start_app_with_postgres(chain_key).await;

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
