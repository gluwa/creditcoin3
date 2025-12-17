use axum::{
    body::Body,
    http::{Request, StatusCode},
};
#[path = "test_utils.rs"]
mod test_utils;
use test_utils::{assert_h256_str, start_app_with_postgres};
use tower::util::ServiceExt; // for oneshot helper

#[cfg_attr(not(feature = "anvil-integration"), ignore)]
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
        .expect("Failed to build request");

    // Act
    let response = app.oneshot(request).await.expect("Failed to send request");

    // Assert
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("Failed to read response body");
    use proof_gen_api_server::services::continuity_service::ContinuityResponse;
    let resp: ContinuityResponse =
        serde_json::from_slice(&bytes).expect("Failed to deserialize response");
    assert_eq!(resp.chain_key, chain_key);
    assert_eq!(resp.header_number, header_number);
    assert!(!resp.continuity_proof.blocks.is_empty());

    // lower_endpoint_digest (H256 -> 0x lowercase hex)
    let lower_digest = resp.continuity_proof.lower_endpoint_digest;
    let lower_str = format!("0x{lower_digest:x}");
    assert_h256_str("lower_endpoint_digest", &lower_str);

    // blocks[*].root and blocks[*].digest
    for (i, b) in resp.continuity_proof.blocks.iter().enumerate() {
        let root = format!("0x{:x}", b.merkle_root);
        let digest = format!("0x{:x}", b.digest);
        assert_h256_str(&format!("blocks[{i}].root"), &root);
        assert_h256_str(&format!("blocks[{i}].digest"), &digest);
    }
}
