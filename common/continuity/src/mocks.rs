use crate::rpc::{CcRpcProvider, EthRpcProvider};
use anyhow::Result;
use async_trait::async_trait;
use attestor_primitives::block::Block;
use attestor_primitives::{AttestationCheckpoint, AttestationData, SignedAttestation};
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
            attestation: AttestationData {
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
        // Attestations every 10 blocks (matching DefaultAttestationInterval = 10)
        Ok(vec![
            mk_attestation(10),
            mk_attestation(20),
            mk_attestation(30),
        ])
    }

    async fn get_last_checkpoint(&self, _chain_key: u64) -> Result<Option<AttestationCheckpoint>> {
        // Checkpoints happen every 100 blocks (10 attestations * 10 interval)
        // For testing, we use checkpoint at block 0 (genesis)
        Ok(Some(AttestationCheckpoint {
            block_number: 0,
            digest: H256::from_low_u64_be(0),
        }))
    }

    async fn get_checkpoints_for_chain(
        &self,
        _chain_key: u64,
    ) -> Result<Vec<AttestationCheckpoint>> {
        // For testing, just provide genesis checkpoint
        // In reality, checkpoints would be at 0, 100, 200, etc.
        Ok(vec![AttestationCheckpoint {
            block_number: 0,
            digest: H256::from_low_u64_be(0),
        }])
    }

    async fn get_checkpoint_by_height(
        &self,
        _chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        // Mock implementation: return checkpoint if it exists in our mock data
        match block_number {
            0 => Ok(Some(AttestationCheckpoint {
                block_number: 0,
                digest: H256::from_low_u64_be(0),
            })),
            _ => Ok(None),
        }
    }

    async fn get_attestation_chain_genesis_block_number(&self, _chain_key: u64) -> Result<u64> {
        // Mock returns 0 for backward compatibility, but can be any number
        Ok(0)
    }

    async fn get_chain_name(&self) -> Result<String> {
        // Mock returns a test chain name
        Ok("Mock CC3 Chain".to_string())
    }
}

/// Mock ETH provider building continuity blocks with simple incremental digests.
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
            // Fake digest: keccak-like by XORing bytes (not cryptographically accurate, just deterministic)
            let mut bytes = [0u8; 32];
            bytes[..16].copy_from_slice(&prev.as_bytes()[..16]);
            bytes[16..24].copy_from_slice(&(n.to_be_bytes()));
            let digest = H256::from(bytes);
            let block = Block {
                block_number: n,
                root,
                prev_digest: prev,
                digest,
            };
            prev = digest;
            blocks.push(block);
        }
        Ok(blocks)
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        // Return a deterministic list of transaction payload bytes for the block
        // For testing, create N transactions where N = (block_number % 3) + 1
        let count = (block_number % 3) as usize + 1;
        let mut txs = Vec::with_capacity(count);
        for i in 0..count {
            // Simple deterministic payload: block_number || tx_index
            let mut b = Vec::with_capacity(16);
            b.extend_from_slice(&block_number.to_be_bytes());
            b.extend_from_slice(&(i as u64).to_be_bytes());
            txs.push(b);
        }
        Ok(txs)
    }

    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>> {
        // Generate a deterministic hash for testing purposes
        // Hash is based on block_number and tx_index
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&block_number.to_be_bytes());
        bytes[8..16].copy_from_slice(&tx_index.to_be_bytes());
        // Fill rest with deterministic pattern
        for (i, byte) in bytes.iter_mut().enumerate().skip(16) {
            *byte = ((block_number + tx_index + i as u64) % 256) as u8;
        }
        Ok(Some(H256::from(bytes)))
    }

    async fn get_tx_position_by_hash(&self, _tx_hash: H256) -> Result<(u64, u64)> {
        Err(anyhow::anyhow!(
            "MockEthRpcProvider does not implement get_tx_position_by_hash"
        ))
    }

    async fn get_last_block(&self) -> Result<u64> {
        // Mock returns a high block number for testing
        Ok(1000)
    }

    async fn get_chain_id(&self) -> Result<u64> {
        // Mock returns test chain ID
        Ok(31337)
    }
}

pub fn make_mock_providers(chain_key: u64) -> (Arc<MockCcRpcProvider>, Arc<MockEthRpcProvider>) {
    (
        Arc::new(MockCcRpcProvider { chain_key }),
        Arc::new(MockEthRpcProvider),
    )
}
