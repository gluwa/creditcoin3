use async_trait::async_trait;
use attestor_primitives::block::Block;
use axum::{body::Body, http::Request};
use continuity::rpc::EthRpcProvider;
use continuity::{ContinuityBuilder, ContinuityConfig};
use proof_gen_api_server::services::mock_providers::MockCcRpcProvider;
mod integration_common;
use integration_common::start_db;
use sp_core::H256;
use std::sync::Arc;
use tower::util::ServiceExt; // oneshot

// Custom ETH provider returning continuity blocks but zero transactions for any block.
struct EmptyTxEthProvider;

#[async_trait]
impl EthRpcProvider for EmptyTxEthProvider {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> anyhow::Result<Vec<Block>> {
        let mut prev = lower_digest;
        let mut blocks = Vec::new();
        for n in start..=end {
            let root = H256::from_low_u64_be(n + 4000);
            let mut bytes = [0u8; 32];
            bytes[..16].copy_from_slice(&prev.as_bytes()[..16]);
            bytes[16..24].copy_from_slice(&n.to_be_bytes());
            let digest = H256::from(bytes);
            blocks.push(Block {
                block_number: n,
                root,
                prev_digest: prev,
                digest,
            });
            prev = digest;
        }
        Ok(blocks)
    }
    async fn get_block_tx_bytes(&self, _block_number: u64) -> anyhow::Result<Vec<Vec<u8>>> {
        Ok(vec![]) // empty transaction list
    }
}

#[tokio::test]
async fn tx_endpoint_empty_block_merkle_proof() {
    let chain_key = 2u64;
    let header_number = 10u64; // between mock attestations

    // Mock CC provider for attestations
    let cc_provider = Arc::new(MockCcRpcProvider { chain_key });
    let eth_provider = Arc::new(EmptyTxEthProvider);

    let config = ContinuityConfig {
        cc3_rpc_url: "ws://mock".into(),
        eth_rpc_url: "ws://mock".into(),
        chain_key,
    };
    let builder = ContinuityBuilder::new_with_providers(config, cc_provider, eth_provider);

    // Start real Postgres and build app with our custom providers
    let db = start_db().await;
    let service = Arc::new(proof_gen_api_server::ContinuityService::new(
        Arc::new(builder),
        Arc::new(db),
    ));
    let app = proof_gen_api_server::build_app(service);

    // tx_index=0 accepted for empty tx list
    let uri = format!("/api/v1/proof/{}/{}/{}", chain_key, header_number, 0);
    let request = Request::builder()
        .uri(uri)
        .method("GET")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(request).await.expect("request failed");
    assert_eq!(response.status().as_u16(), 200);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read body");
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json parse");

    // merkle_proof should exist with empty siblings
    let merkle = &json["merkle_proof"];
    assert!(merkle.is_object(), "merkle_proof present");
    let siblings = merkle["siblings"].as_array().expect("siblings array");
    assert!(siblings.is_empty(), "siblings must be empty for zero txs");
    let root_str = merkle["root"].as_str().expect("root present");
    assert!(root_str.starts_with("0x"), "root hex prefix");
}
