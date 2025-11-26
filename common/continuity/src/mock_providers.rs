use crate::rpc::{CcRpcProvider, EthRpcProvider};
use anyhow::Result;
use async_trait::async_trait;
use attestor_primitives::block::Block;
use attestor_primitives::{Attestation, AttestationCheckpoint, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;
use std::sync::Arc;

/// Mock Creditcoin RPC provider returning deterministic fake attestations & checkpoints.
pub struct MockCcRpcProvider {
    pub chain_key: u64,
}

#[async_trait]
impl CcRpcProvider for MockCcRpcProvider {
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        let mk_attestation = |header_number: u64| SignedAttestation {
            attestation: Attestation {
                chain_key,
                header_number,
                header_hash: H256::from_low_u64_be(header_number),
                root: H256::from_low_u64_be(header_number + 1000),
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

/// Mock ETH provider building continuity blocks with deterministic digests.
pub struct MockEthRpcProvider;

#[async_trait]
impl EthRpcProvider for MockEthRpcProvider {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>> {
        let mut prev = lower_digest;
        let mut blocks = Vec::new();
        for n in start..=end {
            let root = H256::from_low_u64_be(n + 2000);
            // Use the shared hashing helper to mirror production behavior.
            let digest = Block::hash_payload(&n, &root, &prev);
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

pub fn make_mock_providers(chain_key: u64) -> (Arc<MockCcRpcProvider>, Arc<MockEthRpcProvider>) {
    (
        Arc::new(MockCcRpcProvider { chain_key }),
        Arc::new(MockEthRpcProvider),
    )
}
