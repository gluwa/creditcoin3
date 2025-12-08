/// Assert a string is a strict 0x-prefixed, lowercase H256 hex.
pub fn assert_h256_str(label: &str, s: &str) {
    assert!(s.starts_with("0x"), "{label} must start with 0x: {s}");
    assert_eq!(
        s.len(),
        66,
        "{label} must be 0x + 64 hex chars (len=66), got len={}",
        s.len()
    );
    assert!(
        s.chars()
            .skip(2)
            .all(|c| c.is_ascii_hexdigit() && (c.is_ascii_lowercase() || c.is_ascii_digit())),
        "{label} must be lowercase hex (0-9a-f). Got: {s}"
    );
}

// E2E-only helpers. These are compiled only when the `e2e-tests` feature
// is enabled so that the heavy testcontainers / cast dependencies are
// conditional and regular test runs remain lightweight.
#[allow(dead_code)]
mod e2e {
    use anyhow::Result;
    use proof_gen_api_server::db::DbManagerConfig;
    use std::process::{Command, Stdio};

    use axum::Router;
    use continuity::{ContinuityBuilder, ContinuityConfig};
    use proof_gen_api_server::{build_app, ContinuityService};
    use serde_json::Value;
    use std::sync::Arc;
    use testcontainers::runners::AsyncRunner;
    use testcontainers::ContainerAsync;
    use testcontainers_modules::postgres::Postgres;

    /// Send a simple tx using Foundry's cast; returns the tx hash string.
    pub fn send_test_tx_via_cast(port: u16) -> Result<String> {
        let rpc = format!("http://127.0.0.1:{port}");
        // Use the first default Anvil private key (account 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266)
        let anvil_private_key =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
        let mut cmd =
            Command::new(std::env::var("CAST_BIN").unwrap_or_else(|_| "cast".to_string()));
        cmd.arg("send")
            .arg("0x0000000000000000000000000000000000000000")
            .arg("--value")
            .arg("0")
            .arg("--private-key")
            .arg(anvil_private_key)
            .arg("--rpc-url")
            .arg(rpc)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd.spawn()?.wait_with_output()?;
        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "cast send failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);

        // Look for the transactionHash line specifically (not blockHash which appears first)
        for line in stdout.lines() {
            if line.contains("transactionHash") {
                // Extract the hash from "transactionHash      0x..."
                if let Some(pos) = line.find("0x") {
                    let hash_start = pos;
                    let remaining = &line[hash_start..];
                    // Take first 66 characters (0x + 64 hex)
                    if remaining.len() >= 66 {
                        let tx_hash = &remaining[..66];
                        return Ok(tx_hash.to_string());
                    }
                }
            }
        }

        Err(anyhow::anyhow!(
            "failed to find transactionHash in cast output: {}",
            stdout
        ))
    }

    /// Query tx info via JSON-RPC, returning (block_number, tx_index).
    /// Retries up to 20 times with 100ms delay if the transaction isn't mined yet.
    pub async fn get_tx_info_via_rpc(port: u16, tx_hash: &str) -> Result<(u64, u64)> {
        let rpc = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::new();

        // Retry up to 20 times with 100ms delay (total 2 second wait)
        // Anvil should mine instantly, but we allow some buffer for RPC propagation
        for attempt in 0..20 {
            let payload = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionByHash",
                "params": [tx_hash],
                "id": 1,
            });
            let resp = client.post(&rpc).json(&payload).send().await?;
            if !resp.status().is_success() {
                return Err(anyhow::anyhow!("rpc status: {}", resp.status()));
            }
            let v: Value = resp.json().await?;
            let result = v.get("result");

            // Check if result is null (transaction not found)
            if result.is_none() || result.unwrap().is_null() {
                if attempt < 19 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    continue;
                } else {
                    return Err(anyhow::anyhow!(
                        "Transaction {} not found after 20 attempts",
                        tx_hash
                    ));
                }
            }

            let result = result.unwrap();

            // Check if blockNumber exists (transaction is mined)
            if let Some(block_hex) = result.get("blockNumber").and_then(|x| x.as_str()) {
                let txi_hex = result
                    .get("transactionIndex")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| anyhow::anyhow!("no transactionIndex"))?;
                let block_number = u64::from_str_radix(block_hex.trim_start_matches("0x"), 16)?;
                let tx_index = u64::from_str_radix(txi_hex.trim_start_matches("0x"), 16)?;
                return Ok((block_number, tx_index));
            }

            // Transaction found but not mined yet, wait and retry
            if attempt < 19 {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }

        Err(anyhow::anyhow!(
            "Transaction {} not mined after 20 attempts (2 seconds). \
             Anvil might not be running or might not be mining blocks.",
            tx_hash
        ))
    }

    /// Start a Postgres container and return the container handle.
    pub async fn setup_test_postgres() -> ContainerAsync<Postgres> {
        Postgres::default().start().await.expect("start postgres")
    }

    /// Get DbManager config from the running Postgres container.
    pub async fn test_db_manager_config(container: &ContainerAsync<Postgres>) -> DbManagerConfig {
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("get postgres port");
        DbManagerConfig {
            postgres_host: "127.0.0.1".to_string(),
            postgres_port: port.to_string(),
            postgres_user: "postgres".to_string(),
            postgres_password: "postgres".to_string(),
            postgres_db: "postgres".to_string(),
        }
    }

    /// Starts a typed Postgres container and runs migrations, returning an axum Router.
    /// Uses continuity mock providers for CC and ETH.
    /// The container is intentionally leaked to keep Postgres alive for the test duration.
    pub async fn start_app_with_postgres(chain_key: u64) -> Router {
        let container = setup_test_postgres().await;

        let cfg = ContinuityConfig {
            cc3_rpc_url: "ws://mock".into(),
            cc3_key: "//Alice".into(),
            eth_rpc_url: "ws://mock".into(),
            chain_key,
        };
        let (cc_provider, eth_provider) = continuity::mocks::make_mock_providers(chain_key);
        let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider, eth_provider);
        let db_config = test_db_manager_config(&container).await;
        let db = proof_gen_api_server::db::DbManager::new(db_config).expect("db manager init");
        db.run_migrations().await.expect("migrations");
        let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
        std::mem::forget(container);
        build_app(service, chain_key)
    }
}

#[allow(unused_imports)]
pub use e2e::{
    get_tx_info_via_rpc, send_test_tx_via_cast, setup_test_postgres, start_app_with_postgres,
    test_db_manager_config,
};
