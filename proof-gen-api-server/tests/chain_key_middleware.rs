use axum::{
    body::Body,
    http::{Request, StatusCode},
    routing::get,
    Router,
};
use proof_gen_api_server::networking::middleware::chain_key_validator_middleware;
use tower::util::ServiceExt;

#[tokio::test]
async fn test_chain_key_validation_success() {
    let configured_chain_key = 2u64;
    let app = Router::new()
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(|| async { "ok" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            configured_chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = configured_chain_key;
                async move { chain_key_validator_middleware(request, next, chain_key).await }
            },
        ));

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
async fn test_chain_key_validation_failure() {
    let configured_chain_key = 2u64;
    let app = Router::new()
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(|| async { "ok" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            configured_chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = configured_chain_key;
                async move { chain_key_validator_middleware(request, next, chain_key).await }
            },
        ));

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
async fn test_chain_key_validation_with_tx_index() {
    let configured_chain_key = 42u64;
    let app = Router::new()
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(|| async { "ok" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            configured_chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = configured_chain_key;
                async move { chain_key_validator_middleware(request, next, chain_key).await }
            },
        ));

    // Valid chain_key should pass
    let request = Request::builder()
        .uri("/api/v1/proof/42/100/5")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Invalid chain_key should return 400
    let request = Request::builder()
        .uri("/api/v1/proof/1/100/5")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_chain_key_validation_with_tx_hash() {
    let configured_chain_key = 123u64;
    let app = Router::new()
        .route(
            "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
            get(|| async { "ok" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            configured_chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = configured_chain_key;
                async move { chain_key_validator_middleware(request, next, chain_key).await }
            },
        ));

    // Valid chain_key should pass
    let request = Request::builder()
        .uri("/api/v1/proof-by-tx/123/0xabcdef")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Invalid chain_key should return 400
    let request = Request::builder()
        .uri("/api/v1/proof-by-tx/999/0xabcdef")
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_health_endpoint_bypasses_validation() {
    let configured_chain_key = 2u64;
    let app = Router::new()
        .route("/api/v1/health", get(|| async { "healthy" }))
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(|| async { "ok" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            configured_chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = configured_chain_key;
                async move { chain_key_validator_middleware(request, next, chain_key).await }
            },
        ));

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
    let app = Router::new()
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(|| async { "ok" }),
        )
        .layer(axum::middleware::from_fn_with_state(
            configured_chain_key,
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let chain_key = configured_chain_key;
                async move { chain_key_validator_middleware(request, next, chain_key).await }
            },
        ));

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
