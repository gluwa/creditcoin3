//! Shared test utilities for integration tests using alloy.

#[allow(dead_code)]
mod anvil_integration {
    use alloy::network::{EthereumWallet, TransactionBuilder};
    use alloy::primitives::{Address, U256};
    use alloy::providers::ProviderBuilder;
    use alloy::rpc::types::request::TransactionRequest;
    use alloy::signers::local::PrivateKeySigner;
    use alloy_node_bindings::AnvilInstance;

    use anyhow::Result;
    use axum::Router;
    use continuity::{ContinuityBuilder, ContinuityConfig};
    use proof_gen_api_server::{build_app, prom::NoopMetrics, ContinuityService};
    use serde_json::Value;
    use std::sync::Arc;

    /// Sends a test transaction to Anvil and returns the transaction hash.
    pub async fn send_test_tx_via_alloy(port: u16, anvil: &AnvilInstance) -> Result<String> {
        // RPC endpoint URL for embedded Anvil
        let rpc_url = format!("http://127.0.0.1:{port}");
        let url = rpc_url
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid RPC URL '{rpc_url}': {e}"))?;

        // Build Provider with wallet using Anvil's first account
        let signer = PrivateKeySigner::from(anvil.keys()[0].clone());
        let provider = ProviderBuilder::new()
            .wallet(EthereumWallet::from(signer))
            .on_http(url);

        let from = anvil.addresses()[0];
        let to = Address::ZERO;

        // Transaction parameters - sending minimal value for testing
        const TEST_VALUE_WEI: u64 = 1;

        // Build transaction request
        let tx = TransactionRequest::default()
            .with_from(from)
            .with_to(to)
            .with_value(U256::from(TEST_VALUE_WEI));

        // Send transaction and get receipt
        use alloy::providers::Provider as _;
        let pending = provider
            .send_transaction(tx)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to send transaction to Anvil: {e}"))?;

        let receipt = pending
            .get_receipt()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get transaction receipt from Anvil: {e}"))?;

        // Ensure transaction was successful
        if !receipt.status() {
            return Err(anyhow::anyhow!("Transaction failed: {receipt:?}"));
        }

        Ok(format!("{:#x}", receipt.transaction_hash))
    }
    /// Queries transaction info via JSON-RPC, returning (block_number, tx_index).
    pub async fn get_tx_info_via_rpc(port: u16, tx_hash: &str) -> Result<(u64, u64)> {
        let rpc = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::new();

        // Validate input transaction hash format
        if !tx_hash.starts_with("0x") || tx_hash.len() != 66 {
            return Err(anyhow::anyhow!(
                "Invalid transaction hash format: {tx_hash}. Expected 0x + 64 hex characters."
            ));
        }

        // Retry up to 20 times with 100ms delay (total 2 second wait)
        // Anvil should mine instantly, but we allow some buffer for RPC propagation
        // Constants for polling configuration
        const MAX_ATTEMPTS: usize = 20;
        const POLL_INTERVAL_MS: u64 = 100;
        const TOTAL_WAIT_TIME_MS: u64 = MAX_ATTEMPTS as u64 * POLL_INTERVAL_MS; // 2 seconds

        for attempt in 0..MAX_ATTEMPTS {
            let payload = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionByHash",
                "params": [tx_hash],
                "id": 1,
            });

            let resp = client.post(&rpc).json(&payload).send().await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to connect to Anvil RPC at {rpc}. \
                     Ensure Anvil is running and accessible. Error: {e}"
                )
            })?;

            if !resp.status().is_success() {
                return Err(anyhow::anyhow!(
                    "Anvil RPC returned error status: {}. Check Anvil logs for issues.",
                    resp.status()
                ));
            }

            let v: Value = resp.json().await.map_err(|e| {
                anyhow::anyhow!("Failed to parse JSON response from Anvil RPC: {e}")
            })?;

            // Check for JSON-RPC error
            if let Some(error) = v.get("error") {
                return Err(anyhow::anyhow!("Anvil RPC returned error: {error}"));
            }

            let result = v.get("result");

            // Check if result is null (transaction not found)
            if result.is_none_or(|r| r.is_null()) {
                if attempt < MAX_ATTEMPTS - 1 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
                    continue;
                } else {
                    return Err(anyhow::anyhow!(
                        "Transaction {tx_hash} not found after {MAX_ATTEMPTS} attempts ({TOTAL_WAIT_TIME_MS} ms). \
                         Verify the transaction hash is correct and the transaction was submitted to the right chain."
                    ));
                }
            }

            let result = result.expect("result was validated to be Some above");

            if let Some(block_hex) = result.get("blockNumber").and_then(|x| x.as_str()) {
                let txi_hex = result
                    .get("transactionIndex")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Transaction {tx_hash} found but missing transactionIndex field. \
                         This indicates a malformed response from Anvil."
                        )
                    })?;

                let block_number = u64::from_str_radix(block_hex.trim_start_matches("0x"), 16)
                    .map_err(|e| {
                        anyhow::anyhow!("Invalid blockNumber hex format '{block_hex}': {e}")
                    })?;
                let tx_index =
                    u64::from_str_radix(txi_hex.trim_start_matches("0x"), 16).map_err(|e| {
                        anyhow::anyhow!("Invalid transactionIndex hex format '{txi_hex}': {e}")
                    })?;

                return Ok((block_number, tx_index));
            }

            // Transaction found but not mined yet, wait and retry
            if attempt < MAX_ATTEMPTS - 1 {
                tokio::time::sleep(tokio::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
            }
        }

        Err(anyhow::anyhow!(
            "Transaction {tx_hash} not mined after {MAX_ATTEMPTS} attempts ({TOTAL_WAIT_TIME_MS} ms). \
             Anvil might not be mining blocks automatically. \
             Check Anvil configuration and logs."
        ))
    }

    /// Starts test app with mock providers.
    pub async fn start_test_app(chain_key: u64) -> Router {
        let cfg = ContinuityConfig::builder()
            .cc3_rpc_url("ws://mock")
            .eth_rpc_url("ws://mock")
            .chain_key(chain_key)
            .attestation_interval(10)
            .checkpoint_interval(10)
            .build();

        let (cc_provider, eth_provider) = continuity::mocks::make_mock_providers(chain_key);
        let builder = ContinuityBuilder::new_with_providers(cfg, cc_provider.clone(), eth_provider);
        let service = Arc::new(
            ContinuityService::new(Arc::new(builder), None)
                .await
                .expect("service init"),
        );
        build_app(service, chain_key, NoopMetrics::new())
    }

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
}

#[allow(unused_imports)]
pub use anvil_integration::{
    assert_h256_str, get_tx_info_via_rpc, send_test_tx_via_alloy, start_test_app,
};
