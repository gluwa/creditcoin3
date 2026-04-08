//! RPC provider traits for Creditcoin3 and source chain interactions.
//!
//! This module defines abstract traits for RPC operations required by the
//! continuity builder. Implementations exist for real RPC clients, and mock
//! implementations are provided for testing.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::{AccountId32, Client as CcClient};
use eth::continuity::Manager as ContinuityManager;
use sp_core::H256;
use std::sync::Arc;
use user::prelude::*;

use attestor_primitives::block::Block;
use usc_abi_encoding::common::EncodingVersion;
use utils::block_item_traits::BlockItem;

/// Abstraction over Creditcoin3 RPC operations.
///
/// This trait defines all CC3 chain operations required for continuity proof generation.
/// It's implemented by `cc_client::Client` and can be mocked for testing.
///
/// # Implementation
///
/// The production implementation delegates to `cc_client::Client`, which uses the
/// Substrate RPC client to query the CC3 chain.
///
/// # Implementation
///
/// Implemented by `cc_client::Client`. See the trait methods for usage.
#[async_trait]
pub trait CcRpcProvider: Send + Sync {
    /// Fetch all attestations for a chain.
    ///
    /// Returns all attestations currently stored in the CC3 chain's retention buffer.
    /// This is used when no indexer is available.
    async fn get_attestations_for_chain(
        &self,
        chain_key: u64,
    ) -> Result<Vec<SignedAttestation<H256, AccountId32>>>;

    /// Get the most recent checkpoint for a chain.
    async fn get_last_checkpoint(&self, chain_key: u64) -> Result<Option<AttestationCheckpoint>>;

    /// Fetch all checkpoints for a chain.
    ///
    /// Checkpoints provide long-term storage of attestation data and occur
    /// at regular intervals (e.g., every 10 attestations).
    async fn get_checkpoints_for_chain(&self, chain_key: u64)
        -> Result<Vec<AttestationCheckpoint>>;

    /// Get a specific checkpoint by block height.
    async fn get_checkpoint_by_height(
        &self,
        chain_key: u64,
        block_number: u64,
    ) -> Result<Option<AttestationCheckpoint>>;

    /// Get the attestation genesis block number.
    ///
    /// This is the first source chain block that can be attested to.
    /// Blocks before this cannot have proofs generated.
    async fn get_attestation_chain_genesis_block_number(&self, chain_key: u64) -> Result<u64>;

    /// Get the last attestation digest for a chain.
    ///
    /// This is a lightweight query that only fetches the digest without
    /// the full attestation data.
    async fn fetch_last_digest(&self, chain_key: u64) -> Result<Option<H256>>;

    /// Get a specific attestation by its digest.
    ///
    /// Used in combination with `fetch_last_digest` to efficiently query
    /// the last attestation without fetching all attestations.
    async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>>;

    /// Get the attestation interval for a chain.
    ///
    /// Returns the number of source chain blocks between attestations.
    /// For example, if attestations occur every 10 blocks, this returns `10`.
    async fn get_attestation_interval(&self, chain_key: u64) -> Result<Option<u64>>;

    /// Get the checkpoint interval for a chain.
    ///
    /// Returns the number of attestations between checkpoints.
    /// For example, if checkpoints occur every 10 attestations, this returns `10`.
    async fn get_checkpoint_interval(&self, chain_key: u64) -> Result<Option<u64>>;
}

/// Abstraction over source chain (Ethereum/EVM) RPC operations.
///
/// This trait defines all source chain operations required for building
/// continuity fragments. It's implemented by `eth::Client` and can be mocked for testing.
///
/// # Implementation
///
/// The production implementation uses Alloy to interact with Ethereum-compatible chains.
///
/// # Examples
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use continuity::EthRpcProvider;
/// use eth::Client;
///
/// let client = Client::new("https://eth-mainnet.infura.io/v3/YOUR_KEY", None).await?;
/// let last_block = client.get_last_block().await?;
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait EthRpcProvider: Send + Sync {
    /// Build a sequence of continuity blocks from the source chain.
    ///
    /// This is the core operation for building proofs - it fetches blocks from
    /// the source chain and computes the continuity chain with proper digests.
    ///
    /// # Arguments
    ///
    /// * `lower_digest` - The digest to link from (prev_digest of first block)
    /// * `start` - First block number to include
    /// * `end` - Last block number to include
    ///
    /// # Returns
    ///
    /// A vector of blocks forming a valid continuity chain.
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>>;

    /// Fetch raw transaction bytes for a block.
    ///
    /// Returns transaction data in canonical order (as they appear in the block).
    /// Used for building merkle proofs.
    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>>;

    /// Get the transaction hash at a specific index.
    ///
    /// # Returns
    ///
    /// `Some(hash)` if the transaction exists, `None` if index is out of bounds.
    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>>;

    /// Resolve a transaction hash to its position.
    ///
    /// # Returns
    ///
    /// `Some((block_number, tx_index))` if the transaction exists on chain,
    /// `None` if the transaction hash is not found.
    async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<Option<(u64, u64)>>;

    /// Get the current source chain block height.
    async fn get_last_block(&self) -> Result<u64>;

    /// Get the source chain ID.
    ///
    /// Useful for validation and health checks.
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

    async fn fetch_last_digest(&self, chain_key: u64) -> Result<Option<H256>> {
        self.fetch_last_digest(chain_key)
            .await
            .map(|opt| opt.map(|d| H256::from_slice(d.as_bytes())))
            .map_err(|e| anyhow!("Failed to fetch last digest: {e}"))
    }

    async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>> {
        let sp_digest = sp_core::H256::from_slice(digest.as_bytes());
        self.get_attestation_by_digest(chain_key, sp_digest)
            .await
            .map(|opt| {
                opt.map(|att| SignedAttestation {
                    attestation: att.attestation,
                    signature: att.signature,
                    attestors: att.attestors,
                    continuity_proof: att.continuity_proof,
                })
            })
            .map_err(|e| anyhow!("Failed to fetch attestation by digest: {e}"))
    }

    async fn get_attestation_interval(&self, chain_key: u64) -> Result<Option<u64>> {
        self.chain_attestation_interval(chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch attestation interval: {e}"))
    }

    async fn get_checkpoint_interval(&self, chain_key: u64) -> Result<Option<u64>> {
        self.chain_checkpoint_interval(chain_key)
            .await
            .map_err(|e| anyhow!("Failed to fetch checkpoint interval: {e}"))
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
        // Note: This uses Redis block caching if configured (via ContinuityManager -> eth_client.get_block() -> block_cache.rs)
        let manager = ContinuityManager::new(start, end, self);
        manager
            .create(lower_digest, EncodingVersion::V1)
            .await
            .context("Failed to create continuity blocks")
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        // Use encoding V1 for consistency with continuity payload encoding
        // Note: This uses Redis block caching if configured (via get_block() -> block_cache.rs)
        let ordered = self
            .get_block(block_number, EncodingVersion::V1)
            .await
            .unwrap_interrupt("Not handling user interrupts yet")
            .context("Failed to fetch block transactions")?;

        let tx_bytes: Vec<Vec<u8>> = ordered.items().iter().map(|item| item.to_bytes()).collect();

        Ok(tx_bytes)
    }

    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>> {
        // Note: This uses Redis block caching if configured (via get_block() -> block_cache.rs)
        let ordered = self
            .get_block(block_number, EncodingVersion::V1)
            .await
            .unwrap_interrupt("Not handling user interrupts yet")
            .context("Failed to fetch block")?;

        let tx_hash = ordered.items().get(tx_index as usize).map(|item| {
            // Convert alloy BlockHash to sp_core::H256
            let hash_bytes = item.tx_hash().0;
            H256::from_slice(&hash_bytes)
        });

        Ok(tx_hash)
    }

    async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<Option<(u64, u64)>> {
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

/// Type alias for a shared (Arc-wrapped) Creditcoin3 RPC provider.
///
/// This allows multiple builders or services to share the same CC3 client,
/// avoiding duplicate connections.
///
pub type SharedCcProvider = Arc<dyn CcRpcProvider>;

/// Type alias for a shared (Arc-wrapped) source chain RPC provider.
///
/// This allows multiple builders or services to share the same ETH client,
/// which is especially useful when block caching is enabled.
///
pub type SharedEthProvider = Arc<dyn EthRpcProvider>;
