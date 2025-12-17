use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::{AccountId32, Client as CcClient};
use eth::continuity::Manager as ContinuityManager;
use sp_core::H256;
use std::sync::Arc;

use attestor_primitives::block::Block;
use ccnext_abi_encoding::common::EncodingVersion;
use utils::block_item_traits::BlockItem;

/// Abstraction over Creditcoin RPC required for continuity proof generation.
#[async_trait]
pub trait CcRpcProvider: Send + Sync {
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>>;

    async fn get_last_checkpoint(&self, chain_key: u64) -> Result<Option<AttestationCheckpoint>>;

    async fn get_checkpoints_for_chain(&self, chain_key: u64)
        -> Result<Vec<AttestationCheckpoint>>;

    async fn get_checkpoint_by_height(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationCheckpoint>>;

    async fn get_attestation_chain_genesis_block_number(&self, chain_key: u64) -> Result<u64>;

    /// Get the chain name for health check purposes
    async fn get_chain_name(&self) -> Result<String>;
}

/// Abstraction over source chain (ETH) RPC required to build continuity fragments.
#[async_trait]
pub trait EthRpcProvider: Send + Sync {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>>;

    /// Fetch raw transaction payload bytes for a block number.
    /// Returned vector contains one Vec<u8> per transaction in canonical order.
    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>>;

    /// Get the transaction hash at a specific index in a block.
    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>>;

    /// Resolve a transaction hash to its block number and index within the block.
    async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<(u64, u64)>;

    /// Get the current block height (latest block number).
    async fn get_last_block(&self) -> Result<u64>;

    /// Get the chain ID for health check purposes
    async fn get_chain_id(&self) -> Result<u64>;
}

#[async_trait]
impl CcRpcProvider for CcClient {
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>> {
        self.get_attestations_for_chain(chain_key)
            .await
            .context("Failed to fetch attestations")
    }

    async fn get_last_checkpoint(&self, chain_key: u64) -> Result<Option<AttestationCheckpoint>> {
        self.get_last_checkpoint(chain_key)
            .await
            .context("Failed to fetch last checkpoint")
    }

    async fn get_checkpoints_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<AttestationCheckpoint>> {
        self.get_checkpoints_for_chain(chain_key)
            .await
            .context("Failed to fetch checkpoints")
    }

    async fn get_checkpoint_by_height(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        self.get_checkpoint_by_height(chain_key, block_number)
            .await
            .map_err(|e| anyhow!("Failed to fetch checkpoint by height: {e}"))
    }

    async fn get_attestation_chain_genesis_block_number(&self, chain_key: u64) -> Result<u64> {
        self.get_attestation_chain_genesis_block_number(chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch genesis block number: {e}"))
    }

    async fn get_chain_name(&self) -> Result<String> {
        self.get_chain_name()
            .await
            .map_err(|e| anyhow!("Failed to fetch chain name: {e}"))
    }
}

#[async_trait]
impl EthRpcProvider for eth::Client {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>> {
        let manager = ContinuityManager::new(start, end, self);
        let fragment = manager
            .create(lower_digest, EncodingVersion::V1)
            .await
            .context("Failed to create continuity fragment")?;
        Ok(fragment.blocks().to_vec())
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        // Use encoding V1 for consistency with continuity payload encoding
        let ordered = self
            .get_block(block_number, EncodingVersion::V1)
            .await
            .context("Failed to fetch block transactions")?
            .context("Not handling user interrupts yet")?;

        let tx_bytes: Vec<Vec<u8>> = ordered.items().iter().map(|item| item.to_bytes()).collect();

        Ok(tx_bytes)
    }

    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>> {
        let ordered = self
            .get_block(block_number, EncodingVersion::V1)
            .await
            .context("Failed to fetch block")?
            .context("Not handling user interrupts yet")?;

        let tx_hash = ordered.items().get(tx_index as usize).map(|item| {
            // Convert alloy BlockHash to sp_core::H256
            let hash_bytes = item.tx_hash().0;
            H256::from_slice(&hash_bytes)
        });

        Ok(tx_hash)
    }

    async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<(u64, u64)> {
        self.get_tx_position_by_hash(tx_hash)
            .await
            .context("Failed to resolve tx position")
    }

    async fn get_last_block(&self) -> Result<u64> {
        self.get_last_block()
            .await
            .map_err(|e| anyhow!("Failed to get current block height: {e}"))
    }

    async fn get_chain_id(&self) -> Result<u64> {
        self.get_chain_id()
            .await
            .map_err(|e| anyhow!("Failed to get chain ID: {e}"))
    }
}

/// Simple boxed trait object helpers.
pub type SharedCcProvider = Arc<dyn CcRpcProvider>;
pub type SharedEthProvider = Arc<dyn EthRpcProvider>;
