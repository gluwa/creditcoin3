//! Integration tests using alloy-based Anvil and Postgres.
//!
//! Run with: `cargo test --features integration-tests`

use alloy::node_bindings::{Anvil, AnvilInstance};
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use continuity::{mocks::MockCcRpcProvider, ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::db::DbManager;
use proof_gen_api_server::{build_app, ContinuityService, ErrorResponse};

#[path = "test_utils.rs"]
mod test_utils;
use test_utils::{
    assert_h256_str, get_tx_info_via_rpc, send_test_tx_via_alloy, setup_test_postgres,
    test_db_manager_postgres_uri,
};

/// Spawns an Anvil instance with deterministic accounts.
/// Anvil will automatically bind to a random OS-assigned port.
fn spawn_anvil() -> AnvilInstance {
    let mnemonic =
        "abstract vacuum mammal awkward pudding scene penalty purchase dinner depart evoke puzzle";

    Anvil::new().chain_id(31337).mnemonic(mnemonic).spawn()
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn anvil_integration_tx_hash_flow() -> Result<()> {
    // Arrange: Spawn anvil (Alloy will automatically assign a free port)
    let anvil = spawn_anvil();
    let port = anvil.port();

    // Seed a transaction using alloy (no external dependencies)
    let tx_hash = send_test_tx_via_alloy(port, &anvil).await.expect(
        "Failed to send transaction via alloy. This should work without external dependencies.",
    );
    // Validate tx_hash format to ensure it won't accidentally parse as a number
    assert!(tx_hash.starts_with("0x"), "tx_hash must start with 0x");
    assert_eq!(tx_hash.len(), 66, "tx_hash must be 66 chars (0x + 64 hex)");

    // Send dummy transactions to advance the chain to block 15+
    for _ in 0..15 {
        let _ = send_test_tx_via_alloy(port, &anvil).await;
    }

    // Configure ContinuityBuilder with mock CC and real ETH (Anvil)
    let chain_key = 31337;
    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://unused".into(),
        cc3_key: "//Alice".into(),
        eth_rpc_url: anvil.ws_endpoint(),
        chain_key,
    };

    // Build providers: mock CC, real ETH
    let cc_provider = Arc::new(MockCcRpcProvider {
        chain_key: cfg.chain_key,
    });
    let eth_client = eth::Client::new(&cfg.eth_rpc_url, None)
        .await
        .expect("eth client");
    let eth_provider: Arc<dyn continuity::rpc::EthRpcProvider> = Arc::new(eth_client);
    let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider.clone(), eth_provider);

    // Start ephemeral Postgres via shared helper and keep it alive for test duration
    let container = setup_test_postgres().await;

    let db =
        DbManager::new(test_db_manager_postgres_uri(&container).await).expect("db manager init");
    db.run_migrations().await.expect("migrations");
    let service = Arc::new(ContinuityService::new(
        cc_provider,
        Arc::new(builder),
        Arc::new(db),
    ));

    // Build the app router
    let app: Router = build_app(service, chain_key);

    // Serve app and exercise with reqwest
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind http");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    use proof_gen_api_server::services::continuity_service::ContinuityResponse;

    // --- First: health check endpoint (validate basic server functionality)
    let health_url = format!("{base}/api/v1/health");
    let health_resp = client
        .get(&health_url)
        .send()
        .await
        .expect("http send health");
    assert!(
        health_resp.status().is_success(),
        "health check should return success"
    );
    let health_body = health_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse health json");
    assert_eq!(
        health_body.get("status").and_then(|v| v.as_str()),
        Some("healthy"),
        "health check should return status: healthy"
    );

    // Fetch block and tx index from Anvil so we can exercise clean builder paths first
    let (anvil_block_number, anvil_tx_index) = get_tx_info_via_rpc(port, &tx_hash)
        .await
        .expect("Failed to get transaction info from Anvil via RPC");

    // --- Second: block-level endpoint (ensure pure block builder path runs)
    let block_url = format!("{base}/api/v1/proof/31337/{anvil_block_number}");
    let block_resp = client
        .get(&block_url)
        .send()
        .await
        .expect("http send block");
    if !block_resp.status().is_success() {
        let status = block_resp.status();
        let error_body = block_resp
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read body".to_string());
        panic!("Block endpoint returned error status {status}: {error_body}",);
    }
    let block_body = block_resp.bytes().await.expect("read block body");
    let block_json: ContinuityResponse = serde_json::from_slice(&block_body).expect("json block");
    assert_eq!(block_json.chain_key, 31337);
    assert!(
        !block_json.continuity_proof.roots.is_empty(),
        "block continuity proof must be present"
    );
    // Stronger checks: continuity proof encodes roots starting at (queryHeight - 1).
    let roots = &block_json.continuity_proof.roots;
    assert!(!roots.is_empty(), "block continuity proof must be present");
    let start_block_number = anvil_block_number.saturating_sub(1);
    let last_block_number = start_block_number + (roots.len() as u64 - 1);
    // The proof should end at the next attestation after the query block
    // For block 1, the next attestation is at block 10
    assert_eq!(
        last_block_number, 10,
        "continuity chain must end at next attestation (10)"
    );
    assert!(
        (roots.len() as u64) <= (10 - start_block_number + 1),
        "chain length within expected bounds"
    );

    // --- Third: tx-index endpoint (exercise tx-specific builder path)
    let txi_url = format!("{base}/api/v1/proof/31337/{anvil_block_number}/{anvil_tx_index}");
    let txi_resp = client.get(&txi_url).send().await.expect("http send txi");
    assert!(txi_resp.status().is_success());
    let txi_body = txi_resp.bytes().await.expect("read txi body");
    let txi_json: ContinuityResponse = serde_json::from_slice(&txi_body).expect("json txi");
    if let Some(proof) = &txi_json.merkle_proof {
        assert_h256_str("merkle_root (tx-index)", &format!("0x{:x}", proof.root));
    }
    if let Some(th) = &txi_json.tx_hash {
        assert_h256_str("tx_hash (tx-index)", th);
    }
    let txi_roots = &txi_json.continuity_proof.roots;
    assert!(
        !txi_roots.is_empty(),
        "tx-index continuity proof must be present"
    );
    let txi_start = anvil_block_number.saturating_sub(1);
    let txi_last = txi_start + (txi_roots.len() as u64 - 1);
    // The proof should end at the next attestation after the query block (block 10)
    assert_eq!(
        txi_last, 10,
        "tx-index continuity chain must end at next attestation (10)"
    );
    // Wait for background database insert to complete (insert_proofs_entry spawns async task)
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Ensure tx-index response is cached on second request
    let txi_resp2 = client.get(&txi_url).send().await.expect("http send txi2");
    let txi_body2 = txi_resp2.bytes().await.expect("read txi body2");
    let txi_json2: ContinuityResponse = serde_json::from_slice(&txi_body2).expect("json txi2");
    assert!(txi_json2.cached, "tx-index second call should be cached");

    // --- Fourth: tx-hash endpoint (exercise RPC-resolution-by-hash path)
    let url = format!("{base}/api/v1/proof-by-tx/31337/{tx_hash}");
    let resp = client.get(&url).send().await.expect("http send");
    assert!(resp.status().is_success());
    let body = resp.bytes().await.expect("read body");
    let json: ContinuityResponse = serde_json::from_slice(&body).expect("json");
    assert_eq!(json.chain_key, 31337);
    assert!(!json.continuity_proof.roots.is_empty());
    if let Some(proof) = &json.merkle_proof {
        assert_h256_str("merkle_root", &format!("0x{:x}", proof.root));
    }
    if let Some(th) = &json.tx_hash {
        assert_h256_str("tx_hash", th);
    }

    // Wait for background database insert to complete (insert_proofs_entry spawns async task)
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Ensure tx-hash endpoint is cached on second request
    let resp2 = client.get(&url).send().await.expect("http send2");
    let body2 = resp2.bytes().await.expect("read body2");
    let json2: ContinuityResponse = serde_json::from_slice(&body2).expect("json2");
    assert!(json2.cached, "tx-hash second call should be cached");

    // Teardown: server abort; anvil process handled by RAII on drop
    server.abort();

    // Teardown handled by RAII
    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn anvil_integration_health_check_db_failure() -> Result<()> {
    // Arrange: Start anvil for continuity builder (minimal setup)
    let _anvil = spawn_anvil();

    let chain_key = 31337;
    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://unused".into(),
        cc3_key: "//Alice".into(),
        eth_rpc_url: _anvil.ws_endpoint(),
        chain_key,
    };

    // Build providers: mock CC, real ETH
    let cc_provider = Arc::new(MockCcRpcProvider { chain_key });
    let eth_client = eth::Client::new(&cfg.eth_rpc_url, None)
        .await
        .expect("eth client");
    let eth_provider: Arc<dyn continuity::rpc::EthRpcProvider> = Arc::new(eth_client);
    let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider.clone(), eth_provider);

    // Start ephemeral Postgres via shared helper and keep it alive for test duration
    let pg = setup_test_postgres().await;

    let db = DbManager::new(test_db_manager_postgres_uri(&pg).await).expect("db manager init");
    db.run_migrations().await.expect("migrations");
    let service = Arc::new(ContinuityService::new(
        cc_provider,
        Arc::new(builder),
        Arc::new(db),
    ));

    // Build the app router
    let app: Router = build_app(service, chain_key);

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind http");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Define health check URLs
    let health_url = format!("{base}/api/v1/health");
    let liveness_url = format!("{base}/health/live");
    let readiness_url = format!("{base}/health/ready");

    // Act 1: Verify health is initially healthy
    let health_resp = client.get(&health_url).send().await.expect("http send");
    assert!(health_resp.status().is_success());
    let health_body = health_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse json");
    assert_eq!(
        health_body.get("status").and_then(|v| v.as_str()),
        Some("healthy"),
        "initial health check should be healthy"
    );
    assert_eq!(
        health_body
            .get("database_connected")
            .and_then(|v| v.as_bool()),
        Some(true),
        "database should be connected"
    );

    // Test new liveness endpoint (should always return 200 OK)
    let liveness_resp = client.get(&liveness_url).send().await.expect("http send");
    assert_eq!(
        liveness_resp.status(),
        reqwest::StatusCode::OK,
        "liveness should always return 200 OK"
    );
    let liveness_body = liveness_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse liveness json");
    assert_eq!(
        liveness_body.get("status").and_then(|v| v.as_str()),
        Some("alive"),
        "liveness should return alive status"
    );

    // Test readiness endpoint (should return 200 when all dependencies are healthy)
    let readiness_resp = client.get(&readiness_url).send().await.expect("http send");
    assert_eq!(
        readiness_resp.status(),
        reqwest::StatusCode::OK,
        "readiness should return 200 OK when all dependencies are healthy"
    );
    let readiness_body = readiness_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse readiness json");
    assert_eq!(
        readiness_body.get("status").and_then(|v| v.as_str()),
        Some("ready"),
        "readiness should return ready status when healthy"
    );
    assert_eq!(
        readiness_body
            .get("database_ready")
            .and_then(|v| v.as_bool()),
        Some(true),
        "readiness should report database as ready"
    );
    assert_eq!(
        readiness_body
            .get("cc3_rpc_ready")
            .and_then(|v| v.as_bool()),
        Some(true),
        "readiness should report CC3 RPC as ready"
    );
    assert_eq!(
        readiness_body
            .get("eth_rpc_ready")
            .and_then(|v| v.as_bool()),
        Some(true),
        "readiness should report ETH RPC as ready"
    );

    // Act 2: Stop the Postgres container to simulate DB failure
    drop(pg);

    // Give a moment for connections to fail
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Assert: Health endpoint should now return degraded status
    let degraded_resp = client.get(&health_url).send().await.expect("http send");
    assert!(
        degraded_resp.status().is_success(),
        "health endpoint should still return 200 OK when degraded"
    );
    let degraded_body = degraded_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse json");
    assert_eq!(
        degraded_body.get("status").and_then(|v| v.as_str()),
        Some("degraded"),
        "health check should return degraded when database is unavailable"
    );
    assert_eq!(
        degraded_body
            .get("database_connected")
            .and_then(|v| v.as_bool()),
        Some(false),
        "database_connected should be false"
    );

    // Verify liveness endpoint still works (process is running)
    let degraded_liveness_resp = client.get(&liveness_url).send().await.expect("http send");
    assert_eq!(
        degraded_liveness_resp.status(),
        reqwest::StatusCode::OK,
        "liveness should still return 200 OK even with DB failure"
    );

    // Verify readiness endpoint now returns not ready (DB dependency failed)
    let degraded_readiness_resp = client.get(&readiness_url).send().await.expect("http send");
    assert_eq!(
        degraded_readiness_resp.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "readiness should return 503 when database is unavailable"
    );
    let degraded_readiness_body = degraded_readiness_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse degraded readiness json");
    assert_eq!(
        degraded_readiness_body
            .get("status")
            .and_then(|v| v.as_str()),
        Some("not_ready"),
        "readiness should return not_ready when DB fails"
    );
    assert_eq!(
        degraded_readiness_body
            .get("database_ready")
            .and_then(|v| v.as_bool()),
        Some(false),
        "readiness should report database as not ready"
    );

    // Verify proof counts are zeroed out when DB is unavailable
    let proofs = degraded_body.get("proofs_stored").expect("proofs_stored");
    assert_eq!(
        proofs.get("block_level").and_then(|v| v.as_i64()),
        Some(0),
        "block_level should be 0 when DB unavailable"
    );
    assert_eq!(
        proofs.get("transaction_level").and_then(|v| v.as_i64()),
        Some(0),
        "transaction_level should be 0 when DB unavailable"
    );
    assert_eq!(
        proofs.get("total").and_then(|v| v.as_i64()),
        Some(0),
        "total should be 0 when DB unavailable"
    );

    // Verify other fields still work (cache metrics, uptime)
    assert!(
        degraded_body.get("cache_hits").is_some(),
        "cache_hits should still be present"
    );
    assert!(
        degraded_body.get("cache_misses").is_some(),
        "cache_misses should still be present"
    );
    assert!(
        degraded_body.get("uptime_seconds").is_some(),
        "uptime_seconds should still be present"
    );

    // Teardown
    server.abort();
    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn anvil_integration_health_check_rpc_failure() -> Result<()> {
    // This test validates health check behavior when Anvil (ETH RPC) is stopped
    // but database remains functional (demonstrating "degraded" vs "healthy" status)

    // Arrange: Start anvil for initial setup
    let anvil = spawn_anvil();

    let chain_key = 31337;
    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://unused".into(),
        cc3_key: "//Alice".into(),
        eth_rpc_url: anvil.ws_endpoint(),
        chain_key,
    };

    // Build providers: mock CC, real ETH (will be stopped later)
    let cc_provider = Arc::new(MockCcRpcProvider { chain_key });
    let eth_client = eth::Client::new(&cfg.eth_rpc_url, None)
        .await
        .expect("eth client");
    let eth_provider: Arc<dyn continuity::rpc::EthRpcProvider> = Arc::new(eth_client);
    let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider.clone(), eth_provider);

    // Start Postgres (will remain healthy)
    let pg = setup_test_postgres().await;
    let db = DbManager::new(test_db_manager_postgres_uri(&pg).await).expect("db manager init");
    db.run_migrations().await.expect("migrations");
    let service = Arc::new(ContinuityService::new(
        cc_provider,
        Arc::new(builder),
        Arc::new(db),
    ));
    let app: Router = build_app(service, chain_key);

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind http");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    let health_url = format!("{base}/api/v1/health");
    let readiness_url = format!("{base}/health/ready");

    // Act 1: Verify health is initially healthy (all services up)
    let initial_resp = client.get(&health_url).send().await.expect("http send");
    assert!(initial_resp.status().is_success());
    let initial_body = initial_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse json");
    assert_eq!(
        initial_body.get("status").and_then(|v| v.as_str()),
        Some("healthy"),
        "initial health check should be healthy with all services up"
    );

    // Act 2: Stop Anvil (ETH RPC) to simulate upstream failure
    drop(anvil);

    // Give a moment for connections to fail
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Assert: Health should now be degraded (DB works, but ETH RPC fails)
    let degraded_resp = client.get(&health_url).send().await.expect("http send");
    assert!(
        degraded_resp.status().is_success(),
        "health endpoint should still return 200 OK when degraded by upstream failure"
    );
    let degraded_body = degraded_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse degraded json");
    assert_eq!(
        degraded_body.get("status").and_then(|v| v.as_str()),
        Some("degraded"),
        "health check should return degraded when upstream RPC fails"
    );
    assert_eq!(
        degraded_body
            .get("database_connected")
            .and_then(|v| v.as_bool()),
        Some(true),
        "database should still be connected"
    );

    // Verify readiness endpoint reflects upstream failure
    let degraded_readiness_resp = client.get(&readiness_url).send().await.expect("http send");
    assert_eq!(
        degraded_readiness_resp.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "readiness should return 503 when upstream RPC fails"
    );
    let degraded_readiness_body = degraded_readiness_resp
        .json::<serde_json::Value>()
        .await
        .expect("parse degraded readiness json");
    assert_eq!(
        degraded_readiness_body
            .get("status")
            .and_then(|v| v.as_str()),
        Some("not_ready"),
        "readiness should return not_ready when upstream RPC fails"
    );
    assert_eq!(
        degraded_readiness_body
            .get("database_ready")
            .and_then(|v| v.as_bool()),
        Some(true),
        "database should still be ready"
    );
    assert_eq!(
        degraded_readiness_body
            .get("eth_rpc_ready")
            .and_then(|v| v.as_bool()),
        Some(false),
        "ETH RPC should not be ready after Anvil shutdown"
    );

    // Teardown
    server.abort();
    Ok(())
}

#[cfg_attr(not(feature = "integration-tests"), ignore)]
#[tokio::test]
async fn anvil_integration_unattested_block_error() -> Result<()> {
    // This test validates proper error handling when querying a block that hasn't been attested yet
    // Arrange: Start anvil
    let anvil = spawn_anvil();
    let port = anvil.port();

    // Send transactions to advance beyond the highest attestation
    // MockCcRpcProvider returns attestations at: 0, 10, 20, 30, ...
    // So we'll advance to block 50 and query block 35 (after attestation 30, but before attestation 40)
    for _ in 0..50 {
        let _ = send_test_tx_via_alloy(port, &anvil).await;
    }

    let chain_key = 31337;
    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://unused".into(),
        cc3_key: "//Alice".into(),
        eth_rpc_url: anvil.ws_endpoint(),
        chain_key,
    };

    // Build providers with Mock CC that only has attestations up to block 30
    let cc_provider = Arc::new(MockCcRpcProvider { chain_key });
    let eth_client = eth::Client::new(&cfg.eth_rpc_url, None)
        .await
        .expect("eth client");
    let eth_provider: Arc<dyn continuity::rpc::EthRpcProvider> = Arc::new(eth_client);
    let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider.clone(), eth_provider);

    // Start Postgres
    let pg = setup_test_postgres().await;
    let db = DbManager::new(test_db_manager_postgres_uri(&pg).await).expect("db manager init");
    db.run_migrations().await.expect("migrations");
    let service = Arc::new(ContinuityService::new(
        cc_provider,
        Arc::new(builder),
        Arc::new(db),
    ));
    let app: Router = build_app(service, chain_key);

    // Start HTTP server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind http");
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service()).await.ok();
    });
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Act: Query for block 35 (after attestation 30, but no attestation exists after it yet)
    let block_url = format!("{base}/api/v1/proof/31337/35");
    let resp = client.get(&block_url).send().await.expect("http send");

    // Assert: Should return 503 SERVICE_UNAVAILABLE with meaningful error
    assert_eq!(
        resp.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "unattested block should return 503 SERVICE_UNAVAILABLE"
    );

    let error_body = resp
        .json::<ErrorResponse>()
        .await
        .expect("parse error json");

    // Verify error structure with type-safe deserialization
    assert_eq!(
        error_body.code, "BlockNotReady",
        "error code should be BlockNotReady"
    );

    assert!(error_body.retriable, "error should be marked as retriable");

    assert_eq!(
        error_body.block_number,
        Some(35),
        "error should include the requested block_number"
    );

    assert!(
        error_body.current_block.is_some(),
        "error should include current_block"
    );

    assert!(
        error_body.message.contains("not attested"),
        "error message should mention attestation: {}",
        error_body.message
    );

    // Teardown
    server.abort();
    Ok(())
}
