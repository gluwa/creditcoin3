//! Continuity proof generation module
//!
//! This module handles fetching attestations from the Creditcoin3 chain
//! and building continuity proofs for query verification.
//!
//! Key concepts:
//! - Attestations: Consensus points on the Creditcoin3 chain that anchor block digests
//! - Continuity chains: Sequences of blocks that link a query block to attestations
//! - POC compliance: Chains must start at queryHeight-1 and end at next attestation

mod bounds;
mod build;
mod cc3;
mod cc3_data;
mod common;
mod indexer;

pub use bounds::{BoundsFinder, Cc3BoundsFinder, IndexerBoundsFinder};
pub use build::ContinuityResult;
pub use cc3_data::*;

use crate::{
    config::ContinuityConfig,
    errors::ContinuityError,
    proof::BuiltContinuityProof,
    rpc::{SharedCcProvider, SharedEthProvider},
};
use anyhow::{Context, Result};
use cc_client::Client as CcClient;
use eth::Client as EthClient;
use indexer_client::{AttestationWithProof, IndexerClient};
use sp_core::H256;
use std::sync::Arc;
use tracing::info;

/// Indicates whether a continuity proof ends at an actual attestation or a predicted block.
///
/// This is used to track whether the upper endpoint is:
/// - `True` - An actual attestation that exists on the CC3 chain
/// - `False` - A predicted attestation block (for "eager" proof generation)
///
/// # Examples
///
/// ```rust
/// use continuity::EndsInAttestation;
///
/// let ends_in_attestation = EndsInAttestation::True;
/// let is_attested: bool = ends_in_attestation.into();
/// assert!(is_attested);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndsInAttestation {
    /// The upper endpoint is an actual attestation on the CC3 chain
    True,
    /// The upper endpoint is a predicted attestation block
    False,
}

impl From<EndsInAttestation> for bool {
    fn from(e: EndsInAttestation) -> bool {
        matches!(e, EndsInAttestation::True)
    }
}

impl EndsInAttestation {
    /// Check if the proof ends at an actual attestation.
    pub fn is_attested(&self) -> bool {
        matches!(self, EndsInAttestation::True)
    }

    /// Check if the proof ends at a predicted block.
    pub fn is_predicted(&self) -> bool {
        matches!(self, EndsInAttestation::False)
    }
}

/// Builder for generating continuity proofs.
///
/// The `ContinuityBuilder` is the main entry point for generating continuity proofs
/// that link source chain blocks to Creditcoin3 attestations. It supports multiple
/// data sources and can automatically fall back from indexer to chain queries.
///
/// # Data Sources
///
/// The builder uses the following data sources in order of preference:
///
/// 1. **Indexer** (if configured) - Fast, uses pre-computed proofs
/// 2. **CC3 Chain** - Slower, fetches attestations and builds from source chain
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use continuity::{ContinuityBuilder, ContinuityConfig};
///
/// let config = ContinuityConfig::new(
///     "wss://rpc.creditcoin.network",
///     "//Alice",
///     "https://eth-rpc.example.com",
///     1,
///     10,
/// );
///
/// let builder = ContinuityBuilder::new(config).await?;
///
/// // Build proof for a single block
/// let query = 100;
/// let (lower, upper, _) = builder.get_endpoints(&[query], None).await?;
/// let proof = builder.build_for_single_query(query, lower, upper).await?;
/// # Ok(())
/// # }
/// ```
///
/// ## With Indexer
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use continuity::ContinuityBuilder;
/// use indexer_client::IndexerClient;
/// use std::sync::Arc;
/// # use continuity::ContinuityConfig;
/// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
/// # let cc_client = Arc::new(todo!());
/// # let eth_client = Arc::new(todo!());
///
/// let indexer = Arc::new(IndexerClient::new("https://indexer.example.com".to_string())?);
///
/// let builder = ContinuityBuilder::new_with_indexer(
///     config,
///     cc_client,
///     eth_client,
///     Some(indexer),
/// );
/// # Ok(())
/// # }
/// ```
///
/// ## Batch Queries
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// # use continuity::{ContinuityBuilder, ContinuityConfig};
/// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
/// # let builder = ContinuityBuilder::new(config).await?;
///
/// // Build a single proof covering multiple blocks
/// let queries = vec![100, 105, 110];
/// let (lower, upper, _) = builder.get_endpoints(&queries, None).await?;
/// let proof = builder.build_for_batch_queries(&queries, lower, upper).await?;
///
/// println!("Proof covers blocks {} to {}",
///     proof.blocks.first().unwrap().block_number,
///     proof.blocks.last().unwrap().block_number
/// );
/// # Ok(())
/// # }
/// ```
pub struct ContinuityBuilder {
    /// Configuration including RPC URLs and chain parameters
    pub config: ContinuityConfig,

    /// Last checkpoint block number (optional optimization, updated via events)
    /// When set, queries with block numbers > this value can skip checkpoint checks
    pub(crate) last_checkpoint_block: Arc<tokio::sync::RwLock<Option<u64>>>,

    /// Creditcoin3 RPC provider (abstracted for testing)
    pub(crate) cc_provider: SharedCcProvider,

    /// Source chain (ETH/EVM) RPC provider (abstracted for testing)
    pub(crate) eth_provider: SharedEthProvider,

    /// Optional indexer provider for fast proof fetching
    pub(crate) indexer_provider: Option<Arc<IndexerClient>>,
}

impl ContinuityBuilder {
    /// Create a new builder with real RPC clients.
    ///
    /// This is the simplest constructor - it creates RPC clients from the configuration
    /// and uses them to build proofs. No indexer or caching is enabled.
    ///
    /// # Arguments
    ///
    /// * `config` - Configuration including RPC URLs and chain parameters
    ///
    /// # Returns
    ///
    /// Returns a builder configured with live RPC clients.
    ///
    /// # Errors
    ///
    /// Fails if unable to connect to CC3 or ETH RPC endpoints.
    ///
    /// # Note
    ///
    /// Uses read-only CC3 client since signing is not needed for proof generation.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// use continuity::{ContinuityBuilder, ContinuityConfig};
    ///
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("wss://rpc.creditcoin.network")
    ///     .eth_rpc_url("https://eth-mainnet.infura.io/v3/YOUR_KEY")
    ///     .chain_key(1)
    ///     .attestation_interval(10)
    ///     .checkpoint_interval(10)
    ///     .build();
    ///
    /// let builder = ContinuityBuilder::new(config).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(config: ContinuityConfig) -> Result<Self> {
        let cc_client = CcClient::new_read_only(&config.cc3_rpc_url)
            .await
            .context("Failed to create CC client")?;
        let eth_client = EthClient::new(&config.eth_rpc_url, None)
            .await
            .context("Failed to create ETH client")?;

        Ok(Self::new_with_providers(
            config,
            Arc::new(cc_client),
            Arc::new(eth_client),
        ))
    }

    /// Create a new builder with Redis-based block caching enabled.
    ///
    /// Block caching can significantly improve performance by caching source chain
    /// blocks in Redis, reducing RPC calls by ~70% for repeated queries.
    ///
    /// # Arguments
    ///
    /// * `config` - Continuity configuration
    /// * `cache_config` - Redis cache configuration
    ///
    /// # Requires
    ///
    /// - `block_cache` feature must be enabled
    /// - Redis server must be running and accessible
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// use continuity::{ContinuityBuilder, ContinuityConfig};
    /// use eth::block_cache::BlockCacheConfig;
    ///
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("wss://rpc.creditcoin.network")
    ///     .eth_rpc_url("https://eth-rpc.example.com")
    ///     .chain_key(1)
    ///     .checkpoint_interval(10)
    ///     .build();
    ///
    /// let cache_config = BlockCacheConfig {
    ///     redis_url: "redis://localhost:6379".to_string(),
    ///     metrics_registry: None,
    /// };
    ///
    /// let builder = ContinuityBuilder::new_with_block_caching(config, cache_config).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "block_cache")]
    pub async fn new_with_block_caching(
        config: ContinuityConfig,
        cache_config: eth::block_cache::BlockCacheConfig,
    ) -> Result<Self> {
        let cc_client = CcClient::new_read_only(&config.cc3_rpc_url)
            .await
            .context("Failed to create CC client")?;

        let eth_client = EthClient::new_with_cache(&config.eth_rpc_url, None, cache_config)
            .await
            .context("Failed to create caching ETH client")?;

        Ok(Self::new_with_providers(
            config,
            Arc::new(cc_client),
            Arc::new(eth_client),
        ))
    }

    /// Create a builder using injected providers.
    ///
    /// This constructor is useful for:
    /// - Testing with mock providers
    /// - Sharing RPC clients across multiple builders
    /// - Custom provider implementations
    ///
    /// # Arguments
    ///
    /// * `config` - Continuity configuration
    /// * `cc_provider` - Creditcoin3 RPC provider (can be mocked)
    /// * `eth_provider` - Source chain RPC provider (can be mocked)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use continuity::{ContinuityBuilder, ContinuityConfig, mocks::make_mock_providers};
    ///
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("http://mock")
    ///     .eth_rpc_url("http://mock")
    ///     .chain_key(1)
    ///     .attestation_interval(10)
    ///     .checkpoint_interval(10)
    ///     .build();
    ///
    /// let (cc_provider, eth_provider) = make_mock_providers(1);
    ///
    /// let builder = ContinuityBuilder::new_with_providers(
    ///     config,
    ///     cc_provider,
    ///     eth_provider,
    ///     );
    /// ```
    pub fn new_with_providers(
        config: ContinuityConfig,
        cc_provider: SharedCcProvider,
        eth_provider: SharedEthProvider,
    ) -> Self {
        let last_checkpoint_block =
            Arc::new(tokio::sync::RwLock::new(config.last_checkpoint_block));
        Self {
            config,
            cc_provider,
            eth_provider,
            indexer_provider: None,
            last_checkpoint_block,
        }
    }

    /// Create a builder with an optional indexer provider.
    ///
    /// This is the recommended constructor for production use as it enables
    /// fast proof generation via the indexer while maintaining fallback to
    /// chain queries if needed.
    ///
    /// # Arguments
    ///
    /// * `config` - Continuity configuration
    /// * `cc_provider` - Creditcoin3 RPC provider
    /// * `eth_provider` - Source chain RPC provider
    /// * `indexer_provider` - Optional indexer for pre-computed proofs
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// use continuity::ContinuityBuilder;
    /// use indexer_client::IndexerClient;
    /// use std::sync::Arc;
    /// # use continuity::ContinuityConfig;
    /// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
    /// # let cc_client = Arc::new(todo!());
    /// # let eth_client = Arc::new(todo!());
    ///
    /// let indexer = Arc::new(IndexerClient::new(
    ///     "https://indexer.example.com/graphql".to_string()
    /// )?);
    ///
    /// let builder = ContinuityBuilder::new_with_indexer(
    ///     config,
    ///     cc_client,
    ///     eth_client,
    ///     Some(indexer),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn new_with_indexer(
        config: ContinuityConfig,
        cc_provider: SharedCcProvider,
        eth_provider: SharedEthProvider,
        indexer_provider: Option<Arc<IndexerClient>>,
    ) -> Self {
        let last_checkpoint_block =
            Arc::new(tokio::sync::RwLock::new(config.last_checkpoint_block));
        Self {
            config,
            cc_provider,
            eth_provider,
            indexer_provider,
            last_checkpoint_block,
        }
    }

    /// Set or update the indexer provider for this builder.
    ///
    /// This allows adding an indexer to an existing builder or swapping indexers.
    ///
    /// # Arguments
    ///
    /// * `indexer_provider` - Optional indexer provider to use
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use continuity::{ContinuityBuilder, ContinuityConfig};
    /// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
    /// use indexer_client::IndexerClient;
    /// use std::sync::Arc;
    ///
    /// let mut builder = ContinuityBuilder::new(config).await?;
    ///
    /// // Add indexer later
    /// let indexer = Arc::new(IndexerClient::new("https://indexer.example.com".to_string())?);
    /// builder = builder.with_indexer(Some(indexer));
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_indexer(mut self, indexer_provider: Option<Arc<IndexerClient>>) -> Self {
        self.indexer_provider = indexer_provider;
        self
    }

    /// Update the last checkpoint block number (called when CheckpointReached events are received).
    ///
    /// This optimization allows skipping checkpoint checks for queries with block numbers
    /// greater than the last checkpoint.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number of the most recent checkpoint
    pub async fn update_last_checkpoint_block(&self, block_number: u64) {
        let mut last_cp = self.last_checkpoint_block.write().await;
        if last_cp.is_none_or(|current| block_number > current) {
            *last_cp = Some(block_number);
        }
    }

    /// Build a continuity proof for a single query block.
    ///
    /// Generates the minimal continuity chain needed to verify the query block.
    /// The chain starts at `queryHeight - 1` and extends to the next attestation
    /// after the query.
    ///
    /// # Arguments
    ///
    /// * `height` - The block height to generate a proof for
    /// * `lower_attestation` - The attestation at or before `height - 1`
    /// * `upper_attestation` - The attestation after `height`
    ///
    /// # Returns
    ///
    /// A continuity proof containing the block chain from `height` to the upper attestation.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - RPC calls fail
    /// - Attestation bounds are invalid
    /// - Required blocks cannot be fetched
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use continuity::{ContinuityBuilder, ContinuityConfig};
    /// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
    /// # let builder = ContinuityBuilder::new(config).await?;
    ///
    /// let query_height = 100;
    /// let (lower, upper, _) = builder.get_endpoints(&[query_height], None).await?;
    /// let proof = builder.build_for_single_query(query_height, lower, upper).await?;
    ///
    /// assert!(!proof.blocks.is_empty());
    /// assert_eq!(proof.blocks.first().unwrap().block_number, query_height);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn build_for_single_query(
        &self,
        height: u64,
        lower_attestation: AttestationWithProof,
        upper_attestation: AttestationWithProof,
    ) -> ContinuityResult<BuiltContinuityProof> {
        info!(
            query_height = height,
            "Building continuity proof for single query"
        );
        self.build_for_heights(&[height], lower_attestation, upper_attestation)
            .await
    }

    /// Build a continuity proof for multiple query blocks (batch).
    ///
    /// Optimizes gas usage by building a single continuity chain that covers
    /// all query heights. The chain spans from `min(queryHeights) - 1` to the
    /// next attestation after `max(queryHeights)`.
    ///
    /// This is more efficient than generating individual proofs when verifying
    /// multiple blocks in the same attestation range.
    ///
    /// # Arguments
    ///
    /// * `query_heights` - The block heights to generate a proof for (must be non-empty)
    /// * `lower_attestation` - The attestation at or before `min(heights) - 1`
    /// * `upper_attestation` - The attestation after `max(heights)`
    ///
    /// # Returns
    ///
    /// A single continuity proof covering all query blocks.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - `query_heights` is empty
    /// - RPC calls fail
    /// - Attestation bounds are invalid
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use continuity::{ContinuityBuilder, ContinuityConfig};
    /// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
    /// # let builder = ContinuityBuilder::new(config).await?;
    ///
    /// let queries = vec![100, 105, 110];
    /// let (lower, upper, _) = builder.get_endpoints(&queries, None).await?;
    /// let proof = builder.build_for_batch_queries(&queries, lower, upper).await?;
    ///
    /// // Single proof covers all three blocks
    /// assert!(proof.blocks.iter().any(|b| b.block_number == 100));
    /// assert!(proof.blocks.iter().any(|b| b.block_number == 105));
    /// assert!(proof.blocks.iter().any(|b| b.block_number == 110));
    /// # Ok(())
    /// # }
    /// ```
    pub async fn build_for_batch_queries(
        &self,
        query_heights: &[u64],
        lower_attestation: AttestationWithProof,
        upper_attestation: AttestationWithProof,
    ) -> ContinuityResult<BuiltContinuityProof> {
        if query_heights.is_empty() {
            return Err(ContinuityError::EmptyQuery);
        }

        let (min, max) = (
            *query_heights.iter().min().unwrap(),
            *query_heights.iter().max().unwrap(),
        );
        info!(
            query_count = query_heights.len(),
            min_height = min,
            max_height = max,
            "Building continuity proof for batch queries"
        );

        self.build_for_heights(query_heights, lower_attestation, upper_attestation)
            .await
    }

    /// Fetch raw transaction bytes for a block.
    ///
    /// Returns the encoded transaction data for all transactions in the specified block,
    /// in canonical order (as they appear in the block).
    ///
    /// # Arguments
    ///
    /// * `block_number` - The source chain block number
    ///
    /// # Returns
    ///
    /// A vector where each element is the raw bytes of one transaction.
    pub async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        self.eth_provider.get_block_tx_bytes(block_number).await
    }

    /// Get the transaction hash at a specific index in a block.
    ///
    /// # Arguments
    ///
    /// * `block_number` - The source chain block number
    /// * `tx_index` - The transaction index within the block (0-based)
    ///
    /// # Returns
    ///
    /// `Some(hash)` if the transaction exists, `None` if the index is out of bounds.
    pub async fn get_tx_hash_by_index(
        &self,
        block_number: u64,
        tx_index: u64,
    ) -> Result<Option<H256>> {
        self.eth_provider
            .get_tx_hash_by_index(block_number, tx_index)
            .await
    }

    /// Resolve a transaction hash to its block number and index.
    ///
    /// # Arguments
    ///
    /// * `tx_hash` - The transaction hash to look up
    ///
    /// # Returns
    ///
    /// A tuple of `(block_number, tx_index)` where the transaction is located.
    ///
    /// # Errors
    ///
    /// Returns an error if the transaction is not found or RPC calls fail.
    pub async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<(u64, u64)> {
        self.eth_provider.get_tx_position_by_hash(tx_hash).await
    }

    /// Get the current source chain block height.
    ///
    /// Returns the latest block number on the source chain.
    pub async fn get_last_block(&self) -> Result<u64> {
        self.eth_provider.get_last_block().await
    }

    /// Get the CC3 chain name.
    ///
    /// Useful for health checks and validation.
    pub async fn get_chain_name(&self) -> Result<String> {
        self.cc_provider
            .get_chain_name()
            .await
            .context("Failed to get CC3 chain name")
    }

    /// Get the source chain ID.
    ///
    /// Useful for health checks and chain validation.
    pub async fn get_eth_chain_id(&self) -> Result<u64> {
        self.eth_provider
            .get_chain_id()
            .await
            .context("Failed to get ETH chain ID")
    }

    /// Get the attestation genesis block for the configured chain.
    ///
    /// This is the first source chain block that can be attested to.
    /// Blocks before this number cannot have continuity proofs generated.
    ///
    /// # Returns
    ///
    /// The genesis block number for the attestation system on this chain.
    pub async fn get_attestation_genesis_block(&self) -> Result<u64> {
        self.cc_provider
            .get_attestation_chain_genesis_block_number(self.config.chain_key)
            .await
            .context("Failed to get attestation genesis block number")
    }

    /// Get the last attested block number for the configured chain.
    ///
    /// # Returns
    ///
    /// - `Some(block_number)` - The highest block that has been attested
    /// - `None` - No attestations exist yet for this chain
    ///
    /// # Note
    ///
    /// Uses lightweight queries (fetch_last_digest + get_attestation_by_digest)
    /// instead of fetching all attestations.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// # use continuity::{ContinuityBuilder, ContinuityConfig};
    /// # let config = ContinuityConfig::builder().cc3_rpc_url("").eth_rpc_url("").chain_key(1).attestation_interval(10).checkpoint_interval(10).build();
    /// # let builder = ContinuityBuilder::new(config).await?;
    ///
    /// match builder.get_last_attested_block().await? {
    ///     Some(block) => println!("Last attested block: {}", block),
    ///     None => println!("No attestations yet"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_last_attested_block(&self) -> Result<Option<u64>> {
        // First, get the last digest (lightweight query)
        let last_digest = self
            .cc_provider
            .fetch_last_digest(self.config.chain_key)
            .await?;

        let Some(digest) = last_digest else {
            return Ok(None); // No attestations yet
        };

        // Then fetch the attestation by digest to get the header_number
        let attestation = self
            .cc_provider
            .get_attestation_by_digest(self.config.chain_key, digest)
            .await?;

        Ok(attestation.map(|a| a.attestation.header_number))
    }
}
