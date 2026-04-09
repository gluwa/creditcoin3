mod test_utils;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use parameterized::parameterized;
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
        .contains("Chain key not configured: 99"));
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
        .contains("Chain key not configured: 999"));
    assert_eq!(body["retriable"], false);
}

/// Two configured chains: middleware should accept requests for both keys (same path as production multi-chain YAML).
#[tokio::test]
async fn test_multi_chain_middleware_accepts_both_configured_keys() {
    let app = test_utils::start_test_app_chains(&[2u64, 11]).await;

    for chain_key in [2u64, 11] {
        let request = Request::builder()
            .uri(format!("/api/v1/proof/{chain_key}/100/0"))
            .method("GET")
            .body(Body::empty())
            .unwrap();

        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "chain_key {chain_key} should be allowed when configured"
        );
    }
}

/// Two configured chains: an unlisted key should still be rejected.
#[tokio::test]
async fn test_multi_chain_middleware_rejects_unconfigured_key() {
    let app = test_utils::start_test_app_chains(&[2u64, 11]).await;

    let request = Request::builder()
        .uri("/api/v1/proof/999/100/0")
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
        .contains("Chain key not configured: 999"));
    assert_eq!(body["retriable"], false);
}

#[tokio::test]
async fn test_health_endpoint_should_return_success() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    let request = Request::builder()
        .uri("/api/v1/health")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_metrics_route_should_return_success() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    let request = Request::builder()
        .uri("/metrics")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_swagger_route_should_redirect() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    let request = Request::builder()
        .uri("/api/swagger")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
}

#[tokio::test]
async fn test_swagger_json_route_should_return_valid_json() {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    let request = Request::builder()
        .uri("/api/swagger/openapi.json")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(response.into_body(), 10240)
        .await
        .unwrap();
    // note: if we can deserialize without error it must be valid json, no?
    let _body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
}

#[tokio::test]
async fn test_invalid_path_should_return_404_not_found() {
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

#[parameterized(path = {
    "/api/v1/health",
    "/api/v1/proof/2/0/0",
    "/api/v1/proof-by-tx/2/0x0000000000000000000000000000000000000000000000000000000000000000",
    "/metrics",
    "/api/swagger",
})]
#[parameterized_macro(tokio::test)]
async fn test_post_method_should_not_be_allowed(path: &str) {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    let request = Request::builder()
        .uri(path)
        .method("POST")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[parameterized(path = {
    "/api/v1/proof/NOT-EXPECTED/0/0",
    "/api/v1/proof/2/NOT-EXPECTED/0",
    "/api/v1/proof/2/0/NOT-EXPECTED",
    "/api/v1/proof/-4/0/0",
    "/api/v1/proof/2/-4/0",
    "/api/v1/proof/2/0/-4",
    "/api/v1/proof-by-tx/NOT_EXPECTED/0x0000000000000000000000000000000000000000000000000000000000000000",
    "/api/v1/proof-by-tx/2/NOT_EXPECTED",
    "/api/v1/proof-by-tx/2/0xUNEXPECTED",
    "/api/v1/proof-by-tx/-4/0x0000000000000000000000000000000000000000000000000000000000000000",
    "/api/v1/proof-by-tx/2/-4",
})]
#[parameterized_macro(tokio::test)]
async fn test_get_calls_with_bogus_input_return_bad_request(path: &str) {
    let configured_chain_key = 2u64;
    let app = test_utils::start_test_app(configured_chain_key).await;

    let request = Request::builder()
        .uri(path)
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
