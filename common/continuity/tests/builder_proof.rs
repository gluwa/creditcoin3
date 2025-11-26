use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use attestor_primitives::block::Block;
use attestor_primitives::{Attestation, AttestationCheckpoint, Query, SignedAttestation};
use cc_client::AccountId32;
use continuity::rpc::{CcRpcProvider, EthRpcProvider};
use continuity::{builder::ContinuityBuilder, config::ContinuityConfig};
use sp_core::H256;

// Simple mock CC RPC provider for testing builder logic.
struct MockCcRpcProvider;

#[async_trait]
impl CcRpcProvider for MockCcRpcProvider {
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        let mk_attestation = |height: u64| SignedAttestation {
            attestation: Attestation {
                chain_key,
                header_number: height,
                header_hash: H256::from_low_u64_be(height),
                root: H256::from_low_u64_be(height + 1000),
                prev_digest: None,
            },
            signature: [0u8; 96],
            attestors: vec![],
            continuity_proof: Default::default(),
        };
        Ok(vec![mk_attestation(5), mk_attestation(15)])
    }

    async fn get_last_checkpoint(&self, _chain_key: u64) -> Result<Option<AttestationCheckpoint>> {
        Ok(Some(AttestationCheckpoint {
            block_number: 15,
            digest: H256::from_low_u64_be(1500),
        }))
    }

    async fn get_checkpoints_for_chain(
        &self,
        _chain_key: u64,
    ) -> Result<Vec<AttestationCheckpoint>> {
        Ok(vec![
            AttestationCheckpoint {
                block_number: 5,
                digest: H256::from_low_u64_be(500),
            },
            AttestationCheckpoint {
                block_number: 15,
                digest: H256::from_low_u64_be(1500),
            },
        ])
    }
}

// Simple mock ETH RPC provider constructing deterministic continuity blocks.
struct MockEthRpcProvider;

#[async_trait]
impl EthRpcProvider for MockEthRpcProvider {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>> {
        let mut prev = lower_digest;
        let mut out = Vec::new();
        for n in start..=end {
            let root = H256::from_low_u64_be(n + 2000);
            // Use the real digest calculation to better mirror production continuity blocks.
            let digest = Block::hash_payload(&n, &root, &prev);
            out.push(Block {
                block_number: n,
                root,
                prev_digest: prev,
                digest,
            });
            prev = digest;
        }
        Ok(out)
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        let count = (block_number % 3) as usize + 1;
        let mut txs = Vec::with_capacity(count);
        for i in 0..count {
            let mut b = Vec::with_capacity(16);
            b.extend_from_slice(&block_number.to_be_bytes());
            b.extend_from_slice(&(i as u64).to_be_bytes());
            txs.push(b);
        }
        Ok(txs)
    }
}

#[tokio::test]
async fn builder_builds_trimmed_continuity_chain_for_single_query() -> Result<()> {
    let chain_key = 2;
    let config = ContinuityConfig {
        chain_key,
        cc3_rpc_url: "http://localhost:1234".to_string(),
        eth_rpc_url: "http://localhost:5678".to_string(),
    };

    let builder = ContinuityBuilder::new_with_providers(
        config,
        Arc::new(MockCcRpcProvider),
        Arc::new(MockEthRpcProvider),
    );

    let query_height = 10;
    let query = Query {
        chain_id: chain_key,
        height: query_height,
        layout_segments: vec![],
    };
    let proof = builder.build_for_single_query(&query).await?;

    // Expect chain starts at queryHeight - 1 (9) and ends at next attestation (15)
    let first = proof.blocks.first().expect("non-empty continuity chain");
    let last = proof.blocks.last().expect("non-empty continuity chain");

    assert_eq!(
        first.block_number,
        query_height - 1,
        "continuity chain must start at queryHeight-1"
    );
    assert_eq!(
        last.block_number, 15,
        "continuity chain must end at next attestation height"
    );
    assert!(
        proof.blocks.len() <= ((15 - (query_height - 1) + 1) as usize),
        "chain length within expected bounds"
    );

    Ok(())
}
