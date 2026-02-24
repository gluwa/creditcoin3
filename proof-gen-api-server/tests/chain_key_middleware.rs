mod test_utils;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tower::util::ServiceExt;

#[tokio::test]
async fn test_route_with_tx_index_should_report_success_with_valid_chain_key() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    // Valid chain_key should pass
    let request = Request::builder()
        .uri("/api/v1/proof/2/100/0")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_route_with_tx_index_should_report_failure_with_invalid_chain_key() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    // Invalid chain_key should return 400
    let request = Request::builder()
        .uri("/api/v1/proof/99/100/0")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "InvalidChainKey");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("expected 2, got 99"));
    assert_eq!(body["retriable"], false);
}

#[tokio::test]
async fn test_route_with_tx_hash_should_report_success_with_valid_chain_key() {
    let configured_chain_key = 123u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    // Valid chain_key should pass
    let request = Request::builder()
        .uri("/api/v1/proof-by-tx/123/0x1234")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();

    // note: providing a valid tx_hash will not result in 200 response b/c
    // there is no active source chain to connect to, therefore assert that
    // we aren't getting an InvalidChainKey error instead !
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "InvalidParameter");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("tx_hash must be 32 bytes, got 2"));
    assert_eq!(body["retriable"], false);
}

#[tokio::test]
async fn test_route_with_tx_hash_should_report_failure_with_invalid_chain_key() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    // Invalid chain_key should return 400
    let request = Request::builder()
        .uri("/api/v1/proof-by-tx/999/0x0000000000000000000000000000000000000000000000000000000000000000")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["code"], "InvalidChainKey");
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("expected 2, got 999"));
    assert_eq!(body["retriable"], false);
}

#[tokio::test]
async fn test_health_endpoint_bypasses_validation() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    // Health endpoint should always pass
    let request = Request::builder()
        .uri("/api/v1/health")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_invalid_path_format() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    // Paths that don't match expected format should pass through
    // (they'll be handled by route matching, not middleware)
    let request = Request::builder()
        .uri("/api/v1/invalid/path")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    // Should return 404 since route doesn't match, not 400 from middleware
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
