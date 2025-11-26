use axum::{body::Body, http::Request};
use continuity::{ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::{build_app, ContinuityService};
use serde_json::Value;
use std::{process::Command, sync::Arc, time::Duration};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::time::sleep;
use tower::ServiceExt; // for .oneshot

// Postgres-backed caching integration test using typed testcontainers Postgres module.
// Skips gracefully if Docker isn't available.
#[tokio::test]
async fn postgres_backed_caching_works() {
    if Command::new("docker")
        .arg("info")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("Skipping postgres_backed_caching_works: Docker not available");
        return;
    }

    // Start typed Postgres container (async runner, no explicit docker client needed)
    let container = Postgres::default().start().await.expect("start postgres");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("map host port");
    // Construct connection components manually
    // Set env vars for DbManager
    std::env::set_var("POSTGRES_HOST", "127.0.0.1");
    std::env::set_var("POSTGRES_PORT", port.to_string());
    std::env::set_var("POSTGRES_USER", "postgres");
    std::env::set_var("POSTGRES_PASSWORD", "postgres");
    std::env::set_var("POSTGRES_DB", "postgres");

    // Proof parameters
    let chain_key = 2u64;
    let header_number = 10u64;
    let tx_index = 0usize;

    let (cc_provider, eth_provider) =
        proof_gen_api_server::mock_providers::make_mock_providers(chain_key);
    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://mock".into(),
        eth_rpc_url: "ws://mock".into(),
        chain_key,
    };
    let builder = Arc::new(ContinuityBuilder::new_with_providers(
        cfg,
        cc_provider,
        eth_provider,
    ));

    // Init DB + migrations
    let db = proof_gen_api_server::db::DbManager::new().expect("db manager init");
    db.run_migrations().await.expect("run migrations");

    let service = Arc::new(ContinuityService::new(builder.clone(), Arc::new(db)));
    let app = build_app(service);

    // First request (not cached)
    let uri = format!("/api/v1/proof/{chain_key}/{header_number}/{tx_index}");
    let req = Request::builder()
        .uri(&uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app
        .clone()
        .oneshot(req)
        .await
        .expect("initial proof request");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "initial proof generation must succeed"
    );
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json["cached"].as_bool().unwrap(),
        "first generation should not be cached"
    );
    let tx_hash = json["tx_hash"]
        .as_str()
        .expect("tx_hash present")
        .to_string();
    assert!(tx_hash.starts_with("0x") && tx_hash.len() == 66);

    // 2. Poll for cached retrieval via /proof-by-tx until row visible (background insert is spawned).
    let uri_hash = format!("/api/v1/proof-by-tx/{chain_key}/{tx_hash}");
    let mut cached_ok = false;
    for _ in 0..15 {
        // up to ~750ms
        let req_hash = Request::builder()
            .uri(&uri_hash)
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let resp_hash = app
            .clone()
            .oneshot(req_hash)
            .await
            .expect("proof-by-tx request");
        if resp_hash.status().as_u16() == 200 {
            let bytes_hash = axum::body::to_bytes(resp_hash.into_body(), 1024 * 1024)
                .await
                .unwrap();
            let json_hash: Value = serde_json::from_slice(&bytes_hash).unwrap();
            if json_hash["cached"].as_bool().unwrap() {
                // Validate key fields match
                assert_eq!(json_hash["chain_key"].as_u64().unwrap(), chain_key);
                assert_eq!(json_hash["header_number"].as_u64().unwrap(), header_number);
                assert_eq!(json_hash["tx_index"].as_u64().unwrap(), tx_index as u64);
                assert_eq!(json_hash["tx_hash"].as_str().unwrap(), tx_hash);
                cached_ok = true;
                break;
            }
        }
        sleep(Duration::from_millis(50)).await;
    }
    assert!(
        cached_ok,
        "expected cached retrieval after background insertion into Postgres"
    );
    // Container is torn down automatically when dropped (end of test scope).
}
