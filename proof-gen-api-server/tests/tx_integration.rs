use axum::{body::Body, http::Request};
use continuity::{ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::{build_app, mock_providers::make_mock_providers, ContinuityService};
use std::sync::Arc;
use tower::util::ServiceExt; // for oneshot helper

#[tokio::test]
async fn tx_endpoint_returns_merkle_and_verifies() {
    // Arrange: mock providers & builder
    let chain_key = 2u64;
    let header_number = 10u64; // falls between mock attestations 5 and 15
    let tx_index = 0usize;
    let (cc_provider, eth_provider) = make_mock_providers(chain_key);
    let config = ContinuityConfig {
        cc3_rpc_url: "ws://mock".into(),
        eth_rpc_url: "ws://mock".into(),
        chain_key,
    };
    let builder =
        ContinuityBuilder::new_with_providers(config, cc_provider.clone(), eth_provider.clone());
    let arc_builder = Arc::new(builder);

    // Set dummy postgres envs so DbManager::new() doesn't panic; tests assume DB is reachable but not used deeply here
    std::env::set_var("POSTGRES_HOST", "localhost");
    std::env::set_var("POSTGRES_PORT", "5432");
    std::env::set_var("POSTGRES_USER", "test");
    std::env::set_var("POSTGRES_PASSWORD", "test");
    std::env::set_var("POSTGRES_DB", "test");

    let db = proof_gen_api_server::db::DbManager::new().expect("DB manager init");
    let service = Arc::new(ContinuityService::new(arc_builder.clone(), Arc::new(db)));
    let app = build_app(service.clone());

    let uri = format!("/api/v1/proof/{}/{}/{}", chain_key, header_number, tx_index);
    let request = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();

    // Act
    let response = app.clone().oneshot(request).await.expect("request failed");
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body read");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("parse json");

    // Extract merkle proof
    let merkle = &json["merkle_proof"];
    assert!(!merkle.is_null(), "merkle_proof should be present");
    let root_str = merkle["root"].as_str().expect("root present");
    let siblings = merkle["siblings"].as_array().expect("siblings array");

    // Reconstruct QueryMerkleProof from returned JSON to verify
    let root = sp_core::H256::from_slice(&hex::decode(root_str.trim_start_matches("0x")).unwrap());
    let mut entries = Vec::new();
    for s in siblings {
        let hash_str = s["hash"].as_str().expect("hash present");
        let hash =
            sp_core::H256::from_slice(&hex::decode(hash_str.trim_start_matches("0x")).unwrap());
        let is_left = s["is_left"].as_bool().unwrap_or(false);
        entries.push(mmr::query_proof::MerkleProofEntry { hash, is_left });
    }
    let qproof = mmr::query_proof::QueryMerkleProof::new(root, entries);

    // Get tx bytes from eth mock via builder
    let txs = arc_builder
        .get_block_tx_bytes(header_number)
        .await
        .expect("tx bytes");
    let tx = txs[tx_index].clone();

    // Validate merkle proof verifies the tx bytes
    assert!(qproof.verify(&tx), "merkle proof should verify tx bytes");
}
