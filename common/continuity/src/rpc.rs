//! RPC provider traits for Creditcoin3 and source chain interactions.
//!
//! This module defines abstract traits for RPC operations required by the
//! continuity builder. Implementations exist for real RPC clients, and mock
//! implementations are provided for testing.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use attestor_primitives::{block::Block, AttestationCheckpoint, SignedAttestation};
use cc_client::{AccountId32, Client as CcClient};
use eth::continuity::Manager as ContinuityManager;
use sp_core::H256;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tracing::warn;
use usc_abi_encoding::common::EncodingVersion;
use user::prelude::*;
use utils::block_item_traits::BlockItem;

/// Total attempts (call + retries-after-reconnect) for a single RPC operation.
const ETH_RPC_MAX_ATTEMPTS: usize = 3;

/// Backoff used when reconnecting the shared client (mirrors the attestor's CC3 reconnect).
const RECONNECT_BACKOFF_BASE_MS: u64 = 100;
const RECONNECT_BACKOFF_MAX_MS: u64 = 5_000;
/// Total reconnect attempts (initial call + retries). `tokio_retry` treats the strategy length
/// as the number of *retries*, so we pass `RECONNECT_MAX_ATTEMPTS - 1` to `.take(..)`.
const RECONNECT_MAX_ATTEMPTS: usize = 5;

/// ETH RPC provider that owns one long-lived [`eth::Client`] and reconnects it on transport
/// failures.
///
/// The pattern mirrors the attestor's CC3 [`ReconnectingRuntimeApi`]: keep a single client,
/// retry the call after reconnecting with exponential backoff + jitter, and surface a clean
/// error if reconnection itself can't recover.
///
/// [`ReconnectingRuntimeApi`]: cc_client::api::ReconnectingRuntimeApi
#[derive(Debug)]
pub struct ReconnectingEthRpcProvider {
    client: RwLock<eth::Client>,
    /// Source-chain block encoding, derived from CC3 supported-chain metadata at
    /// startup. Used for all block fetching / continuity building so that a
    /// per-chain or future encoding change is honoured instead of assuming V1.
    encoding: EncodingVersion,
}

impl ReconnectingEthRpcProvider {
    pub fn new(client: eth::Client, encoding: EncodingVersion) -> Self {
        Self {
            client: RwLock::new(client),
            encoding,
        }
    }

    /// Run an RPC call, reconnecting and retrying on failure.
    ///
    /// `op` is a short identifier (e.g. `"get_chain_id"`) used in tracing.
    async fn run<T, F, Fut>(&self, op: &'static str, mut call: F) -> Result<T>
    where
        F: FnMut(eth::Client) -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let mut last_err: Option<anyhow::Error> = None;

        for attempt in 1..=ETH_RPC_MAX_ATTEMPTS {
            let client = self.client.read().await.clone();
            match call(client).await {
                Ok(value) => return Ok(value),
                Err(err) => {
                    // A user-initiated shutdown (Ctrl+C / service stop) surfaces here as an
                    // `anyhow::Error` carrying `user::Shutdown` (via `propagate_shutdown` in the
                    // block-fetch closures). It is not a transport failure, so do not reconnect
                    // or burn the remaining retry budget — return immediately so the service
                    // exits promptly.
                    if err.chain().any(|cause| cause.is::<user::Shutdown>()) {
                        warn!(
                            op,
                            attempt, "ETH RPC call interrupted by shutdown; not retrying"
                        );
                        return Err(err);
                    }
                    if eth::anyhow_chain_is_inconsistent_block_payload(&err) {
                        warn!(
                            op,
                            attempt,
                            max = ETH_RPC_MAX_ATTEMPTS,
                            error = %err,
                            "ETH RPC returned block data inconsistent with header; not retrying with reconnect",
                        );
                        return Err(err);
                    }
                    warn!(
                        op,
                        attempt,
                        max = ETH_RPC_MAX_ATTEMPTS,
                        error = %err,
                        "ETH RPC call failed",
                    );
                    last_err = Some(err);
                }
            }

            if attempt < ETH_RPC_MAX_ATTEMPTS {
                self.reconnect(op).await?;
            }
        }

        Err(last_err
            .unwrap_or_else(|| anyhow!("no error captured"))
            .context(format!("{op} failed after {ETH_RPC_MAX_ATTEMPTS} attempts")))
    }

    /// Reconnect the shared client with exponential backoff + jitter.
    async fn reconnect(&self, op: &'static str) -> Result<()> {
        // `tokio_retry` runs the action once, then drains the strategy iterator on each retry.
        // Subtract one so `RECONNECT_MAX_ATTEMPTS` reflects total attempts (matches
        // `ETH_RPC_MAX_ATTEMPTS`).
        let strategy = ExponentialBackoff::from_millis(RECONNECT_BACKOFF_BASE_MS)
            .max_delay(Duration::from_millis(RECONNECT_BACKOFF_MAX_MS))
            .map(jitter)
            .take(RECONNECT_MAX_ATTEMPTS.saturating_sub(1));

        tokio_retry::Retry::spawn(strategy, || async {
            warn!(op, "reconnecting ETH RPC client");
            self.client
                .write()
                .await
                .reconnect()
                .await
                .map_err(|e| anyhow!("{e}"))
        })
        .await
        .with_context(|| format!("failed to reconnect ETH RPC client for {op}"))?;

        Ok(())
    }
}

/// Abstraction over Creditcoin3 RPC operations.
///
/// This trait defines all CC3 chain operations required for continuity proof generation.
/// It's implemented by `cc_client::Client` and can be mocked for testing.
///
/// # Implementation
///
/// The production implementation delegates to `cc_client::Client`, which uses the
/// Substrate RPC client to query the CC3 chain.
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
/// continuity fragments. It's implemented by [`ReconnectingEthRpcProvider`] (production)
/// and can be mocked for testing.
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

    /// Fetch a block's transaction bytes **and** the transaction hash at `tx_index` from a
    /// single block fetch, instead of two separate `get_block` round-trips (one for the bytes,
    /// one for the hash). Providers backed by a real RPC override this to fetch the block once;
    /// the default falls back to the two separate calls for mocks / unoptimized providers.
    async fn get_block_tx_bytes_and_tx_hash(
        &self,
        block_number: u64,
        tx_index: u64,
    ) -> Result<(Vec<Vec<u8>>, Option<H256>)> {
        let bytes = self.get_block_tx_bytes(block_number).await?;
        let hash = self.get_tx_hash_by_index(block_number, tx_index).await?;
        Ok((bytes, hash))
    }

    /// Fetch all tx hashes and encoded tx bytes for a block in canonical order.
    async fn get_block_tx_data(&self, block_number: u64) -> Result<Vec<(H256, Vec<u8>)>> {
        let bytes = self.get_block_tx_bytes(block_number).await?;
        let mut txs = Vec::with_capacity(bytes.len());
        for (idx, tx_bytes) in bytes.into_iter().enumerate() {
            if let Some(tx_hash) = self.get_tx_hash_by_index(block_number, idx as u64).await? {
                txs.push((tx_hash, tx_bytes));
            }
        }
        Ok(txs)
    }

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

    /// Check if the source chain RPC is healthy.
    async fn is_healthy(&self) -> Result<bool>;
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
impl EthRpcProvider for ReconnectingEthRpcProvider {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>> {
        let encoding = self.encoding;
        self.run("build_continuity_blocks", move |client| async move {
            ContinuityManager::new(start, end, &client)
                .create(lower_digest, encoding)
                .await
                .context("Failed to create continuity blocks")
        })
        .await
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        let encoding = self.encoding;
        self.run("get_block_tx_bytes", move |client| async move {
            let ordered = client
                .get_block(block_number, encoding)
                .await
                // Propagate a user-initiated `Interrupt::Stop` as a typed `Shutdown` error
                // (carried inside this `anyhow::Error`) instead of panicking, so Ctrl+C /
                // service shutdown exits gracefully.
                .propagate_shutdown::<anyhow::Error>()
                .context("Failed to fetch block transactions")?;

            Ok(ordered.items().iter().map(|item| item.to_bytes()).collect())
        })
        .await
    }

    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>> {
        let encoding = self.encoding;
        self.run("get_tx_hash_by_index", move |client| async move {
            let ordered = client
                .get_block(block_number, encoding)
                .await
                .propagate_shutdown::<anyhow::Error>()
                .context("Failed to fetch block")?;

            Ok(ordered.items().get(tx_index as usize).map(|item| {
                let hash_bytes = item.tx_hash().0;
                H256::from_slice(&hash_bytes)
            }))
        })
        .await
    }

    async fn get_block_tx_bytes_and_tx_hash(
        &self,
        block_number: u64,
        tx_index: u64,
    ) -> Result<(Vec<Vec<u8>>, Option<H256>)> {
        let encoding = self.encoding;
        self.run("get_block_tx_bytes_and_tx_hash", move |client| async move {
            // One block fetch yields both the ordered tx bytes and the tx hash at `tx_index`,
            // replacing the previous two separate `get_block` calls.
            let ordered = client
                .get_block(block_number, encoding)
                .await
                .propagate_shutdown::<anyhow::Error>()
                .context("Failed to fetch block")?;

            let bytes = ordered.items().iter().map(|item| item.to_bytes()).collect();
            let hash = ordered
                .items()
                .get(tx_index as usize)
                .map(|item| H256::from_slice(&item.tx_hash().0));
            Ok((bytes, hash))
        })
        .await
    }

    async fn get_block_tx_data(&self, block_number: u64) -> Result<Vec<(H256, Vec<u8>)>> {
        let encoding = self.encoding;
        self.run("get_block_tx_data", move |client| async move {
            let ordered = client
                .get_block(block_number, encoding)
                .await
                .propagate_shutdown::<anyhow::Error>()
                .context("Failed to fetch block")?;

            Ok(ordered
                .items()
                .iter()
                .map(|item| (H256::from_slice(&item.tx_hash().0), item.to_bytes()))
                .collect())
        })
        .await
    }

    async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<Option<(u64, u64)>> {
        self.run("get_tx_position_by_hash", move |client| async move {
            client
                .get_tx_position_by_hash(tx_hash)
                .await
                .map_err(|e| anyhow!("{e}"))
                .context("Rpc error resolving tx position")
        })
        .await
    }

    async fn get_last_block(&self) -> Result<u64> {
        self.run("get_last_block", |client| async move {
            client
                .get_last_block()
                .await
                .map_err(|e| anyhow!("Failed to get current block height: {e}"))
        })
        .await
    }

    async fn get_chain_id(&self) -> Result<u64> {
        self.run("get_chain_id", |client| async move {
            client
                .get_chain_id()
                .await
                .map_err(|e| anyhow!("Failed to get chain ID: {e}"))
        })
        .await
    }

    async fn is_healthy(&self) -> Result<bool> {
        let _ = self
            .get_chain_id()
            .await
            .map_err(|e| anyhow!("Failed to get chain ID: {e}"))?;

        Ok(true)
    }
}

/// Type alias for a shared (Arc-wrapped) Creditcoin3 RPC provider.
///
/// This allows multiple builders or services to share the same CC3 client,
/// avoiding duplicate connections.
pub type SharedCcProvider = Arc<dyn CcRpcProvider>;

/// Type alias for a shared (Arc-wrapped) source chain RPC provider.
///
/// This allows multiple builders or services to share the same ETH client,
/// which is especially useful when block caching is enabled.
pub type SharedEthProvider = Arc<dyn EthRpcProvider>;
