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
#[cfg(feature = "e2e-tests")]
#[allow(dead_code)]
mod e2e {
    use anyhow::Result;
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
        let rpc = format!("http://127.0.0.1:{}", port);
        let mut cmd =
            Command::new(std::env::var("CAST_BIN").unwrap_or_else(|_| "cast".to_string()));
        cmd.arg("send")
            .arg("0x0000000000000000000000000000000000000000")
            .arg("--value")
            .arg("0")
            .arg("--from")
            .arg("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266")
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
        if let Some(pos) = stdout.find("0x") {
            let end = pos + 66;
            if end <= stdout.len() {
                let candidate = &stdout[pos..end];
                if candidate.len() == 66 {
                    return Ok(candidate.to_string());
                }
            }
        }
        Err(anyhow::anyhow!(
            "failed to parse tx hash from cast output: {}",
            stdout
        ))
    }

    /// Query tx info via JSON-RPC, returning (block_number, tx_index).
    pub async fn get_tx_info_via_rpc(port: u16, tx_hash: &str) -> Result<(u64, u64)> {
        let rpc = format!("http://127.0.0.1:{}", port);
        let client = reqwest::Client::new();
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
        let result = v
            .get("result")
            .ok_or_else(|| anyhow::anyhow!("no result"))?;
        let block_hex = result
            .get("blockNumber")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow::anyhow!("no blockNumber"))?;
        let txi_hex = result
            .get("transactionIndex")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow::anyhow!("no transactionIndex"))?;
        let block_number = u64::from_str_radix(block_hex.trim_start_matches("0x"), 16)?;
        let tx_index = u64::from_str_radix(txi_hex.trim_start_matches("0x"), 16)?;
        Ok((block_number, tx_index))
    }

    /// Start a Postgres container and set POSTGRES_* env vars for DbManager.
    pub async fn setup_postgres_env() -> ContainerAsync<Postgres> {
        let container = Postgres::default().start().await.expect("start postgres");
        let port = container.get_host_port_ipv4(5432).await.expect("host port");
        std::env::set_var("POSTGRES_HOST", "127.0.0.1");
        std::env::set_var("POSTGRES_PORT", port.to_string());
        std::env::set_var("POSTGRES_USER", "postgres");
        std::env::set_var("POSTGRES_PASSWORD", "postgres");
        std::env::set_var("POSTGRES_DB", "postgres");
        container
    }

    /// Starts a typed Postgres container and runs migrations, returning an axum Router.
    /// Uses continuity mock providers for CC and ETH.
    /// The container is intentionally leaked to keep Postgres alive for the test duration.
    pub async fn start_app_with_postgres(chain_key: u64) -> Router {
        let container = Postgres::default().start().await.expect("start postgres");
        let port = container.get_host_port_ipv4(5432).await.expect("host port");
        std::env::set_var("POSTGRES_HOST", "127.0.0.1");
        std::env::set_var("POSTGRES_PORT", port.to_string());
        std::env::set_var("POSTGRES_USER", "postgres");
        std::env::set_var("POSTGRES_PASSWORD", "postgres");
        std::env::set_var("POSTGRES_DB", "postgres");

        let cfg = ContinuityConfig {
            cc3_rpc_url: "ws://mock".into(),
            eth_rpc_url: "ws://mock".into(),
            chain_key,
        };
        let (cc_provider, eth_provider) = continuity_mocks::make_mock_providers(chain_key);
        let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider, eth_provider);
        let db = proof_gen_api_server::db::DbManager::new().expect("db manager init");
        db.run_migrations().await.expect("migrations");
        let service = Arc::new(ContinuityService::new(Arc::new(builder), Arc::new(db)));
        std::mem::forget(container);
        build_app(service)
    }
}

#[cfg(feature = "e2e-tests")]
#[allow(unused_imports)]
pub use e2e::{
    get_tx_info_via_rpc, send_test_tx_via_cast, setup_postgres_env, start_app_with_postgres,
};
