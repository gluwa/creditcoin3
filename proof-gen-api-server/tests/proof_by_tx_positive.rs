use axum::{body::Body, http::Request};
use continuity::{ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::services::mock_providers::make_mock_providers;
use proof_gen_api_server::{build_app, ContinuityService};
use serde_json::Value;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tower::ServiceExt; // oneshot

// Positive flow: generate tx-level proof, then fetch by tx_hash (cached path)
#[tokio::test]
async fn proof_by_tx_positive_cached_retrieval() {
    // Arrange environment vars for DbManager
    std::env::set_var("POSTGRES_HOST", "localhost");
    std::env::set_var("POSTGRES_PORT", "5432");
    std::env::set_var("POSTGRES_USER", "test");
    std::env::set_var("POSTGRES_PASSWORD", "test");
    std::env::set_var("POSTGRES_DB", "test");

    std::env::set_var("INMEM_DB_FALLBACK", "1"); // enable in-memory fallback since Postgres may not be running in test env
    let chain_key = 2u64;
    let header_number = 10u64; // within mock attestation range
    let tx_index = 0u64; // first tx
    let (cc_provider, eth_provider) = make_mock_providers(chain_key);
    let config = ContinuityConfig {
        cc3_rpc_url: "ws://mock".into(),
        eth_rpc_url: "ws://mock".into(),
        chain_key,
    };
    let builder = ContinuityBuilder::new_with_providers(config, cc_provider, eth_provider);
    let db = proof_gen_api_server::db::DbManager::new().expect("db manager init");
    let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
    let app = build_app(service);

    // 1. Generate tx-specific proof (should not be cached on first call)
    let uri = format!("/api/v1/proof/{chain_key}/{header_number}/{tx_index}");
    let req = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app
        .clone()
        .oneshot(req)
        .await
        .expect("tx proof request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "initial tx proof generation must succeed"
    );
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["chain_key"].as_u64().unwrap(), chain_key);
    assert_eq!(json["header_number"].as_u64().unwrap(), header_number);
    assert_eq!(json["tx_index"].as_u64().unwrap(), tx_index);
    let tx_hash = json["tx_hash"]
        .as_str()
        .expect("tx_hash present")
        .to_string();
    assert!(
        tx_hash.starts_with("0x") && tx_hash.len() == 66,
        "expected 32-byte keccak tx hash hex"
    );
    assert!(
        !json["cached"].as_bool().unwrap(),
        "first generation should not be cached"
    );

    // Give background insertion task a brief moment (spawned) to persist the row
    sleep(Duration::from_millis(50)).await;

    // 2. Retrieve via /proof-by-tx using tx_hash (should hit cache)
    let uri_hash = format!("/api/v1/proof-by-tx/{chain_key}/{tx_hash}");
    let req_hash = Request::builder()
        .uri(uri_hash)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp_hash = app
        .clone()
        .oneshot(req_hash)
        .await
        .expect("proof-by-tx request failed");
    assert_eq!(
        resp_hash.status().as_u16(),
        200,
        "proof-by-tx must succeed for existing hash"
    );
    let bytes_hash = axum::body::to_bytes(resp_hash.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json_hash: Value = serde_json::from_slice(&bytes_hash).unwrap();

    // 3. Assertions comparing cached response
    assert_eq!(json_hash["chain_key"].as_u64().unwrap(), chain_key);
    assert_eq!(json_hash["header_number"].as_u64().unwrap(), header_number);
    assert_eq!(json_hash["tx_index"].as_u64().unwrap(), tx_index);
    assert_eq!(json_hash["tx_hash"].as_str().unwrap(), tx_hash);
    assert!(
        json_hash["cached"].as_bool().unwrap(),
        "second retrieval should be cached"
    );

    // Continuity proof consistency (block count and digests match)
    let blocks_initial = json["continuity_proof"]["blocks"].as_array().unwrap();
    let blocks_cached = json_hash["continuity_proof"]["blocks"].as_array().unwrap();
    assert_eq!(
        blocks_initial.len(),
        blocks_cached.len(),
        "continuity blocks length should be identical"
    );
    for (a, b) in blocks_initial.iter().zip(blocks_cached.iter()) {
        assert_eq!(a["root"], b["root"], "roots should match");
        assert_eq!(a["digest"], b["digest"], "digests should match");
    }

    // Merkle proof consistency (root and sibling list length)
    let merkle_initial = json["merkle_proof"]
        .as_object()
        .expect("merkle initial present");
    let merkle_cached = json_hash["merkle_proof"]
        .as_object()
        .expect("merkle cached present");
    assert_eq!(
        merkle_initial.get("root"),
        merkle_cached.get("root"),
        "merkle roots must match"
    );
    let sib_initial = merkle_initial["siblings"].as_array().unwrap();
    let sib_cached = merkle_cached["siblings"].as_array().unwrap();
    assert_eq!(
        sib_initial.len(),
        sib_cached.len(),
        "sibling count should match"
    );
}
