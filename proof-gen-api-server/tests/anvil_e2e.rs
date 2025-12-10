//! E2E test harness using a real Anvil node (ephemeral) to exercise the tx-hash flow.
//!
//! Notes:
//! - Requires `anvil` and `cast` binaries available on PATH (Foundry).
//!   Install with: curl -L https://foundry.paradigm.xyz | bash && foundryup
//! - Marks the test as ignored by default; run with `cargo test -p proof-gen-api-server --tests -- --ignored`.
//! - Test will fail with a clear error message if Foundry tools are not available.

// Use async reqwest for readiness probe and RPC fetches
use anyhow::Result;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

// local helper to setup ephemeral postgres for this test
use axum::Router;
use continuity::{mocks::MockCcRpcProvider, ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::db::DbManager;
use proof_gen_api_server::{build_app, ContinuityService};

// Bring in shared test helpers
#[path = "test_utils.rs"]
mod test_utils;
use test_utils::{
    assert_h256_str, get_tx_info_via_rpc, send_test_tx_via_cast, setup_test_postgres,
    TEST_DB_MANAGER_POSTGRES_URI,
};

/// RAII wrapper to ensure the anvil process is killed on drop.
struct Anvil(Child);

impl Drop for Anvil {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

/// Spawn anvil on a fixed port for test determinism.
/// You can randomize the port if running in parallel.
/// Anvil mines blocks instantly on transaction submission (no --block-time flag).
async fn spawn_anvil(port: u16) -> Anvil {
    let mut cmd = Command::new(std::env::var("ANVIL_BIN").unwrap_or_else(|_| "anvil".to_string()));
    let child = cmd
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--chain-id")
        .arg("31337")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn anvil");

    // Readiness probe: poll Anvil JSON-RPC until eth_chainId returns (async-native)
    let rpc = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_chainId",
        "params": [],
        "id": 1,
    });
    for _ in 0..50u8 {
        if let Ok(resp) = client.post(&rpc).json(&payload).send().await {
            if resp.status().is_success() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Anvil(child)
}

#[cfg_attr(not(feature = "e2e-tests"), ignore)]
#[tokio::test]
async fn e2e_tx_hash_flow_with_anvil() -> Result<()> {
    // Arrange: select a free TCP port for Anvil to improve parallelism
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind probe port");
    let port: u16 = probe.local_addr().unwrap().port();
    drop(probe);
    // Spawn anvil on the chosen port
    let _anvil = spawn_anvil(port).await;

    // Seed a transaction using `cast send` (requires Foundry).
    let tx_hash = send_test_tx_via_cast(port).expect(
        "Failed to send transaction via cast. \
         Ensure 'cast' is installed and available in PATH. \
         Install Foundry with: curl -L https://foundry.paradigm.xyz | bash && foundryup",
    );
    // Validate tx_hash format to ensure it won't accidentally parse as a number
    assert!(tx_hash.starts_with("0x"), "tx_hash must start with 0x");
    assert_eq!(tx_hash.len(), 66, "tx_hash must be 66 chars (0x + 64 hex)");

    // Send dummy transactions to advance the chain to block 15+
    for _ in 0..15 {
        let _ = send_test_tx_via_cast(port);
    }

    // Configure ContinuityBuilder with mock CC and real ETH (Anvil)
    let chain_key = 31337;
    let cfg = ContinuityConfig {
        cc3_rpc_url: "ws://unused".into(),
        cc3_key: "//Alice".into(),
        eth_rpc_url: format!("ws://127.0.0.1:{port}"),
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
    let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider, eth_provider);

    // Start ephemeral Postgres via shared helper and keep it alive for test duration
    setup_test_postgres().await;

    let db = DbManager::new(TEST_DB_MANAGER_POSTGRES_URI.to_string()).expect("db manager init");
    db.run_migrations().await.expect("migrations");
    let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
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
        !block_json.continuity_proof.blocks.is_empty(),
        "block continuity proof must be present"
    );
    // Stronger checks: continuity proof encodes blocks starting at (queryHeight - 1).
    let blocks = &block_json.continuity_proof.blocks;
    assert!(!blocks.is_empty(), "block continuity proof must be present");
    let start_block_number = anvil_block_number.saturating_sub(1);
    let last_block_number = start_block_number + (blocks.len() as u64 - 1);
    // The proof should end at the next attestation after the query block
    // For block 1, the next attestation is at block 10
    assert_eq!(
        last_block_number, 10,
        "continuity chain must end at next attestation (10)"
    );
    assert!(
        (blocks.len() as u64) <= (10 - start_block_number + 1),
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
    let txi_blocks = &txi_json.continuity_proof.blocks;
    assert!(
        !txi_blocks.is_empty(),
        "tx-index continuity proof must be present"
    );
    let txi_start = anvil_block_number.saturating_sub(1);
    let txi_last = txi_start + (txi_blocks.len() as u64 - 1);
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
    assert!(!json.continuity_proof.blocks.is_empty());
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
