use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use hex;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::collections::{BTreeMap, HashMap};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crate::prom::Metrics;
use crate::services::continuity_service::helpers::*;
use attestor_primitives::block::ContinuityProof;
use continuity::ContinuityBuilder;
use merkle::proof::TransactionMerkleProof;
use merkle_cache::MerkleProofCache;

pub mod helpers;
mod merkle_cache;

/// Per-source-chain state: ETH-backed builder and CC3-derived caches.
pub struct ChainState {
    pub builder: Arc<ContinuityBuilder>,
    pub checkpoint_cache: tokio::sync::RwLock<BTreeMap<u64, H256>>,
    pub attestation_cache: tokio::sync::RwLock<BTreeMap<u64, H256>>,
    pub merkle_proof_cache: MerkleProofCache,
    pub attestation_genesis_block: AtomicU64,
}

// Single block proof query object, used in batch requests to specify which transactions to include merkle proofs for. If a block is included in the batch but not listed in the tx_indexes, it will be processed with tx_index = None (continuity proof only)
#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProofQuery {
    pub header_number: u64,
    #[serde(default)]
    pub tx_indexes: Vec<u64>, // Empty vector means no merkle proof, just continuity proof
}

// Merkle proof object. This is what we will enter in the DB and perhaps
// also what we return from api calls
#[derive(Debug, Clone)]
pub struct MerkleProofItem {
    pub chain_key: u64,
    pub header_number: u64,
    pub tx_index: Option<u64>, // Maybe should make this non-null if we remove intended support for full block merkle proofs
    pub tx_hash: Option<H256>,
    pub tx_bytes: Option<Vec<u8>>, // Cached transaction bytes (payload only)
    // Use concrete types for downstream consumers; we'll serialize only at DB boundary.
    pub merkle_proof: TransactionMerkleProof,
    pub merkle_root: H256,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
#[serde(untagged)]
pub enum ContinuityResponse {
    Single(SingleContinuityResponse),
    Batch(BatchedContinuityResponse),
}

impl From<SingleContinuityResponse> for ContinuityResponse {
    fn from(response: SingleContinuityResponse) -> Self {
        ContinuityResponse::Single(response)
    }
}

impl From<BatchedContinuityResponse> for ContinuityResponse {
    fn from(response: BatchedContinuityResponse) -> Self {
        ContinuityResponse::Batch(response)
    }
}

/// Schema for OpenAPI documentation of ContinuityProof.
/// Matches the structure of attestor_primitives::block::ContinuityProof.
#[derive(utoipa::ToSchema)]
#[schema(rename_all = "camelCase")]
pub struct ContinuityProofSchema {
    /// Digest of the block before the continuity chain starts (digest of queryHeight - 1).
    /// 32-byte Keccak256 hash as hex string (e.g. 0x...).
    pub lower_endpoint_digest: String,
    /// Array of merkle roots; digests are computed on-chain.
    /// Block number for index i = startBlock + i, where startBlock = queryBlockHeight.
    /// The query block is at index 0.
    pub roots: Vec<String>,
}

/// Schema for OpenAPI documentation of a single sibling in a Merkle proof path.
#[derive(utoipa::ToSchema)]
#[schema(rename_all = "camelCase")]
pub struct MerkleProofEntrySchema {
    /// The sibling hash (32-byte Keccak256 as hex string).
    pub hash: String,
    /// True if this sibling is to the left of the current hash in the tree.
    pub is_left: bool,
}

/// Schema for OpenAPI documentation of TransactionMerkleProof.
/// Proves that a transaction is included in a block's Merkle tree.
#[derive(utoipa::ToSchema)]
#[schema(rename_all = "camelCase")]
pub struct TransactionMerkleProofSchema {
    /// The Merkle root hash of the block's transaction tree (32-byte Keccak256 as hex string).
    pub root: String,
    /// Sibling hashes along the path from leaf to root, with position information.
    pub siblings: Vec<MerkleProofEntrySchema>,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SingleContinuityResponse {
    pub chain_key: u64,
    pub header_number: u64,
    pub tx_index: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_bytes: Option<String>, // Hex-encoded transaction bytes (payload only)
    #[schema(value_type = ContinuityProofSchema)]
    pub continuity_proof: ContinuityProof,
    #[schema(value_type = TransactionMerkleProofSchema)]
    pub merkle_proof: TransactionMerkleProof,
    pub cached: bool,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchedContinuityResponse {
    pub chain_key: u64,
    pub from_header: u64,
    pub to_header: u64,
    #[schema(value_type = ContinuityProofSchema)]
    pub continuity_proof: ContinuityProof,
    pub merkle_proofs: BTreeMap<u64, BTreeMap<u64, BatchedMerkleProofEntry>>,
    pub cached: bool,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchedMerkleProofEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_bytes: Option<String>, // Hex-encoded transaction bytes (payload only)
    #[schema(value_type = TransactionMerkleProofSchema)]
    pub merkle_proof: TransactionMerkleProof,
}

use crate::services::errors::ServiceError;
pub type ServiceResult<T> = Result<T, ServiceError>;

pub struct ContinuityService {
    chains: HashMap<u64, Arc<ChainState>>,
    start_time: Instant,
    /// Prometheus metrics for instrumentation (uses NoopMetrics when disabled).
    metrics: Metrics,
    /// Maximum amount of concurrent futures spawned when generating proofs for batch requests or when extracting transaction indexes from transaction hashes.
    max_batch_size: usize,
    /// Maximum allowed span (highest block − lowest block) in a single batch
    /// request.  Prevents a small batch from forcing proof generation over an
    /// extremely large block range.
    max_batch_span: u64,
}

const MERKLE_BACKFILL_POLL_INTERVAL: Duration = Duration::from_secs(15);
const MERKLE_BACKFILL_MAX_BLOCKS_PER_TICK: usize = 50;
const MERKLE_BACKFILL_MAX_CONCURRENCY: usize = 8;
const MERKLE_PROOF_CACHE_CHECKPOINT_RETENTION_MULTIPLIER: u64 = 4;

impl ContinuityService {
    /// Create a new ContinuityService, fetching the attestation genesis block from the chain.
    ///
    /// # Errors
    /// Returns an error if the attestation genesis block cannot be fetched from RPC.
    pub async fn new(
        builders: Vec<Arc<ContinuityBuilder>>,
        metrics: Metrics,
        max_batch_size: usize,
        max_batch_span: u64,
    ) -> anyhow::Result<Self> {
        if builders.is_empty() {
            anyhow::bail!("ContinuityService requires at least one ContinuityBuilder");
        }

        let mut chains = HashMap::new();
        for builder in builders {
            let chain_key = builder.config.chain_key;
            if chains.contains_key(&chain_key) {
                anyhow::bail!("duplicate ContinuityBuilder for chain_key {chain_key}");
            }

            // Fetch genesis block at startup - fail fast if RPC is unavailable
            tracing::debug!(
                chain_key,
                "🚀 🔗 [startup] ContinuityService: fetching attestation genesis block from CC3"
            );
            let attestation_genesis_block = builder
                .get_attestation_genesis_block()
                .await
                .with_context(|| {
                    format!(
                        "Failed to fetch attestation genesis block during ContinuityService init (chain_key={chain_key}); CC3 RPC used by this builder may be down or misconfigured"
                    )
                })?;

            tracing::debug!(
                chain_key,
                attestation_genesis_block,
                "🚀 ✅ ContinuityService chain initialized with attestation genesis block"
            );

            // Populate checkpoint cache from CC3 on startup.
            tracing::debug!(
                chain_key,
                "🚀 ⏳ 📝 Populating checkpoint cache from CC3 (this may take a while)..."
            );
            let checkpoints = builder
                .cc_provider
                .get_checkpoints_for_chain(chain_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        chain_key,
                        "⚠️ 🔗 Failed to fetch checkpoints on startup: {e}, starting with empty cache"
                    );
                    Vec::new()
                });
            let checkpoint_map: BTreeMap<u64, H256> = checkpoints
                .into_iter()
                .map(|cp| (cp.block_number, cp.digest))
                .collect();
            tracing::debug!(
                chain_key,
                count = checkpoint_map.len(),
                latest = ?checkpoint_map.keys().next_back(),
                "🚀 ✅ 📝 Checkpoint cache populated from CC3"
            );
            metrics
                .set_last_checkpoint_height(chain_key, checkpoint_map.keys().next_back().copied());

            // Populate attestation cache from CC3 on startup.
            tracing::debug!(
                chain_key,
                "🚀 ⏳ 📜 Populating attestation cache from CC3 (this may take a while)..."
            );
            let attestations = builder
                .cc_provider
                .get_attestations_for_chain(chain_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        chain_key,
                        "⚠️ 🔗 Failed to fetch attestations on startup: {e}, starting with empty cache"
                    );
                    Vec::new()
                });
            let attestation_map: BTreeMap<u64, H256> = attestations
                .into_iter()
                .map(|att| (att.attestation.header_number, att.attestation.digest()))
                .collect();
            tracing::debug!(
                chain_key,
                count = attestation_map.len(),
                latest = ?attestation_map.keys().next_back(),
                "🚀 ✅ 📜 Attestation cache populated from CC3"
            );
            let latest_attested_height = attestation_map.keys().next_back().copied();
            metrics.set_last_attested_height(chain_key, latest_attested_height);

            chains.insert(
                chain_key,
                Arc::new(ChainState {
                    builder,
                    checkpoint_cache: tokio::sync::RwLock::new(checkpoint_map),
                    attestation_cache: tokio::sync::RwLock::new(attestation_map),
                    merkle_proof_cache: MerkleProofCache::default(),
                    attestation_genesis_block: AtomicU64::new(attestation_genesis_block),
                }),
            );
        }

        Ok(Self {
            chains,
            start_time: Instant::now(),
            metrics,
            max_batch_size,
            max_batch_span,
        })
    }

    pub(crate) fn chain_state(&self, chain_key: u64) -> ServiceResult<&Arc<ChainState>> {
        self.chains
            .get(&chain_key)
            .ok_or(ServiceError::UnknownChain { chain_key })
    }

    pub(crate) fn serves_chain(&self, chain_key: u64) -> bool {
        self.chains.contains_key(&chain_key)
    }

    pub(crate) fn configured_chain_keys(&self) -> std::collections::HashSet<u64> {
        self.chains.keys().copied().collect()
    }

    pub fn spawn_merkle_backfill(service: Arc<Self>) {
        for chain in service.chains.values().cloned().collect::<Vec<_>>() {
            let service = service.clone();
            tokio::spawn(async move {
                tracing::info!(
                    chain_key = chain.builder.config.chain_key,
                    poll_interval_secs = MERKLE_BACKFILL_POLL_INTERVAL.as_secs(),
                    max_blocks_per_tick = MERKLE_BACKFILL_MAX_BLOCKS_PER_TICK,
                    max_concurrency = MERKLE_BACKFILL_MAX_CONCURRENCY,
                    "merkle proof cache backfill worker started"
                );
                loop {
                    if let Err(err) = service.backfill_merkle_cache_for_chain(chain.clone()).await {
                        tracing::warn!(
                            chain_key = chain.builder.config.chain_key,
                            error = %err,
                            "merkle proof cache backfill tick failed"
                        );
                    }
                    tokio::time::sleep(MERKLE_BACKFILL_POLL_INTERVAL).await;
                }
            });
        }
    }

    async fn backfill_merkle_cache_for_chain(&self, chain: Arc<ChainState>) -> anyhow::Result<()> {
        let chain_key = chain.builder.config.chain_key;
        let (_, confirmed_tip) = chain.builder.get_confirmed_last_block().await?;
        let Some(latest_attested_height) = Self::cached_attested_height(chain.as_ref()).await
        else {
            tracing::info!(
                chain_key,
                confirmed_tip,
                "merkle proof cache backfill waiting for attested height"
            );
            return Ok(());
        };
        let cache_tip = latest_attested_height.min(confirmed_tip);
        let retention_blocks = self.merkle_cache_retention_blocks(chain.as_ref());
        let genesis = chain.attestation_genesis_block.load(Ordering::Relaxed);
        let start = cache_tip.saturating_sub(retention_blocks).max(genesis);

        if start > cache_tip {
            return Ok(());
        }

        let heights = chain
            .merkle_proof_cache
            .unprocessed_heights_desc(start, cache_tip, MERKLE_BACKFILL_MAX_BLOCKS_PER_TICK)
            .await;
        if heights.is_empty() {
            tracing::info!(
                chain_key,
                confirmed_tip,
                latest_attested_height,
                cache_tip,
                retained_from = start,
                retention_blocks,
                "merkle proof cache backfill already warm for retained range"
            );
            return Ok(());
        }

        use futures::StreamExt as _;

        let concurrency = self
            .max_batch_size
            .clamp(1, MERKLE_BACKFILL_MAX_CONCURRENCY);
        let results = futures::stream::iter(heights.into_iter().map(|height| {
            let chain = chain.clone();
            async move {
                let result = self.precompute_merkle_cache_block(&chain, height).await;
                (height, result)
            }
        }))
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;

        let mut cached_blocks = 0usize;
        let mut cached_txs = 0usize;
        let mut failed_blocks = 0usize;
        let mut min_height = u64::MAX;
        let mut max_height = 0u64;

        for (height, result) in results {
            min_height = min_height.min(height);
            max_height = max_height.max(height);
            match result {
                Ok(tx_count) => {
                    cached_blocks += 1;
                    cached_txs += tx_count;
                    tracing::debug!(
                        chain_key,
                        height,
                        tx_count,
                        "merkle proof cache backfilled source block"
                    );
                }
                Err(err) => {
                    failed_blocks += 1;
                    tracing::warn!(
                        chain_key,
                        height,
                        error = %err,
                        "failed to backfill merkle proof cache block"
                    );
                }
            }
        }

        tracing::info!(
            chain_key,
            confirmed_tip,
            latest_attested_height,
            cache_tip,
            retained_from = start,
            min_height,
            max_height,
            cached_blocks,
            cached_txs,
            failed_blocks,
            "merkle proof cache backfill tick completed"
        );

        let min_retained = cache_tip.saturating_sub(retention_blocks).max(genesis);
        let removed = chain.merkle_proof_cache.prune_below(min_retained).await;
        if removed > 0 {
            tracing::info!(
                chain_key,
                min_retained,
                removed,
                "pruned old merkle proof cache blocks after backfill"
            );
        }

        Ok(())
    }

    async fn precompute_merkle_cache_block(
        &self,
        chain: &Arc<ChainState>,
        header_number: u64,
    ) -> ServiceResult<usize> {
        let txs = chain
            .builder
            .get_block_tx_data(header_number)
            .await
            .map_err(|err| map_eth_rpc_anyhow_to_service_error(err, header_number))?;

        if txs.is_empty() {
            chain
                .merkle_proof_cache
                .mark_processed_empty(header_number)
                .await;
            return Ok(0);
        }

        Ok(chain
            .merkle_proof_cache
            .insert_block(header_number, txs)
            .await)
    }

    async fn precompute_merkle_cache_block_for_tx(
        &self,
        chain: &Arc<ChainState>,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<Option<MerkleProofItem>> {
        let txs = chain
            .builder
            .get_block_tx_data(header_number)
            .await
            .map_err(|err| map_eth_rpc_anyhow_to_service_error(err, header_number))?;

        if txs.is_empty() {
            chain
                .merkle_proof_cache
                .mark_processed_empty(header_number)
                .await;
            return Ok(None);
        }

        let (tx_count, item) = chain
            .merkle_proof_cache
            .insert_block_and_get(chain.builder.config.chain_key, header_number, txs, tx_index)
            .await;

        tracing::info!(
            chain_key = chain.builder.config.chain_key,
            header_number,
            tx_index,
            tx_count,
            cache_hit = item.is_some(),
            "merkle proof cache on-demand fill completed"
        );

        Ok(item)
    }

    fn merkle_cache_retention_blocks(&self, chain: &ChainState) -> u64 {
        chain
            .builder
            .config
            .checkpoint_block_interval()
            .saturating_mul(MERKLE_PROOF_CACHE_CHECKPOINT_RETENTION_MULTIPLIER)
    }

    /// Update the continuity builder's last-checkpoint hint (from on-chain events).
    pub async fn update_builder_last_checkpoint(&self, chain_key: u64, block_number: u64) {
        if let Some(chain) = self.chains.get(&chain_key) {
            chain
                .builder
                .update_last_checkpoint_block(block_number)
                .await;
        }
    }

    /// Validate that the requested blocks can be processed:
    /// 1. Not before attestation genesis
    /// 2. Exists on source chain (ETH)
    ///
    /// Returns the current block height for reuse in validating predicted attestation bounds.
    async fn validate_blocks(
        &self,
        chain: &Arc<ChainState>,
        header_numbers: &[u64],
    ) -> ServiceResult<u64> {
        let chain_key = chain.builder.config.chain_key;
        let genesis_block = chain.attestation_genesis_block.load(Ordering::Acquire);

        // Check genesis bound
        if let Some(&header_number) = header_numbers.iter().find(|h| **h <= genesis_block) {
            tracing::warn!(
                requested_block = header_number,
                genesis_block,
                chain_key,
                "⚠️  Requested block is before or at attestation genesis"
            );
            return Err(ServiceError::BlockBeforeOrAtGenesis {
                requested_block: header_number,
                genesis_block,
            });
        }

        // Check source chain existence, applying block_confirmation_depth for reorg protection.
        // Blocks within `block_confirmation_depth` of the tip are considered unconfirmed and
        // are rejected the same way as blocks that don't exist yet.
        // get_confirmed_last_block() returns (tip, confirmed) in a single RPC call.
        let (tip_block, confirmed_block) =
            chain
                .builder
                .get_confirmed_last_block()
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get current block height from source chain: {e}"),
                })?;

        if let Some(&header_number) = header_numbers.iter().find(|h| **h > confirmed_block) {
            tracing::warn!(
                requested_block = header_number,
                tip_block,
                confirmed_block,
                chain_key,
                block_confirmation_depth = chain.builder.config.block_confirmation_depth,
                "⚠️  ⛓️ Requested block is not yet confirmed on source chain (within reorg window)"
            );
            return Err(ServiceError::BlockNotOnSourceChain {
                requested_block: header_number,
                current_block: tip_block,
                confirmation_depth: chain.builder.config.block_confirmation_depth,
            });
        }
        let current_block = confirmed_block;

        Ok(current_block)
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Health check for CC3 RPC connectivity
    pub async fn check_cc3_connectivity(&self) -> anyhow::Result<()> {
        // Use a lightweight storage query as a connectivity check (any configured chain)
        let (_, chain) = self
            .chains
            .iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no chains configured"))?;
        let _genesis = chain.builder.get_attestation_genesis_block().await?;
        Ok(())
    }

    /// Health check for ETH RPC connectivity
    pub async fn check_eth_connectivity(&self) -> anyhow::Result<()> {
        for (chain_key, chain) in &self.chains {
            chain
                .builder
                .eth_provider
                .is_healthy()
                .await
                .map_err(|e| anyhow::anyhow!("eth RPC chain {chain_key}: {e}"))?
                .then_some(())
                .ok_or_else(|| anyhow::anyhow!("eth RPC chain {chain_key} is unhealthy"))?;
        }
        Ok(())
    }

    /// Returns the latest attested height in the in-memory cache for a chain.
    pub async fn attested_height(&self, chain_key: u64) -> ServiceResult<Option<u64>> {
        let chain = self.chain_state(chain_key)?;

        Ok(Self::cached_attested_height(chain.as_ref()).await)
    }

    /// Insert an attestation into the in-memory cache (called from event handler).
    pub async fn insert_attestation(&self, chain_key: u64, block_number: u64, digest: H256) {
        if let Some(chain) = self.chains.get(&chain_key) {
            {
                chain
                    .attestation_cache
                    .write()
                    .await
                    .insert(block_number, digest);
            }
            tracing::debug!(
                chain_key,
                block_number,
                ?digest,
                "🔧 📦 📜 attestation cached"
            );
            self.metrics.set_last_attested_height(
                chain_key,
                Self::cached_attestation_height(chain.as_ref()).await,
            );
        }
    }

    async fn cached_attestation_height(chain: &ChainState) -> Option<u64> {
        let cache = chain.attestation_cache.read().await;
        cache.keys().next_back().copied()
    }

    async fn cached_attested_height(chain: &ChainState) -> Option<u64> {
        let mut last_height = {
            let cache = chain.attestation_cache.read().await;
            cache.keys().next_back().copied()
        };

        if last_height.is_none() {
            let checkpoint_cache = chain.checkpoint_cache.read().await;
            last_height = checkpoint_cache.keys().next_back().copied();
        }

        last_height
    }

    async fn cached_checkpoint_height(chain: &ChainState) -> Option<u64> {
        let checkpoint_cache = chain.checkpoint_cache.read().await;
        checkpoint_cache.keys().next_back().copied()
    }

    /// Truncate attestation and checkpoint caches to the given revert height.
    /// Removes all entries with block numbers strictly greater than `revert_height`.
    /// Called when a `RevertedAttestationChainTo` event is received from CC3.
    pub async fn revert_caches(&self, chain_key: u64, revert_height: u64) {
        if let Some(chain) = self.chains.get(&chain_key) {
            // split_off is O(log n) on BTreeMap vs O(n) for retain.
            let split_key = revert_height.saturating_add(1);

            {
                let mut att = chain.attestation_cache.write().await;
                let removed = att.split_off(&split_key).len();
                tracing::info!(
                    chain_key,
                    revert_height,
                    removed,
                    remaining = att.len(),
                    "🔧 ↩️  Reverted attestation cache"
                );
            }

            {
                let mut cp = chain.checkpoint_cache.write().await;
                let removed = cp.split_off(&split_key).len();
                tracing::info!(
                    chain_key,
                    revert_height,
                    removed,
                    remaining = cp.len(),
                    "🔧 ↩️  Reverted checkpoint cache"
                );
            }

            let removed_merkle = chain.merkle_proof_cache.prune_above(revert_height).await;
            if removed_merkle > 0 {
                tracing::info!(
                    chain_key,
                    revert_height,
                    removed = removed_merkle,
                    "🔧 ↩️  Reverted merkle proof cache"
                );
            }

            // Reset the builder's last-checkpoint hint so that
            // determine_checkpoint_info does not skip checks based on stale state.
            chain.builder.reset_last_checkpoint_block().await;
            self.metrics.set_last_attested_height(
                chain_key,
                Self::cached_attestation_height(chain.as_ref()).await,
            );
            self.metrics.set_last_checkpoint_height(
                chain_key,
                Self::cached_checkpoint_height(chain.as_ref()).await,
            );
        }
    }

    /// Drop attestation cache entries at or below `height`.
    ///
    /// Called after a `CheckpointReached` event. Once a checkpoint absorbs a
    /// run of attestations, the on-chain pallet appends them to a per-chain
    /// removal queue (`pallets/attestation/src/impls.rs` `remove_attestations`)
    /// and evicts the oldest entries once the queue exceeds
    /// `AttestationRetentionDuration`. Any of those consumed attestations that
    /// we still hold in cache will eventually point at a digest the on-chain
    /// verifier no longer recognizes, producing "Continuity proof does not
    /// match attestation or checkpoint" at verification time. Pruning at the
    /// checkpoint boundary keeps the cache aligned with the post-checkpoint
    /// invariant: attestations strictly above the latest checkpoint are the
    /// only ones the verifier will accept as proof anchors.
    pub async fn prune_attestations_at_or_below(&self, chain_key: u64, height: u64) {
        if let Some(chain) = self.chains.get(&chain_key) {
            {
                let split_key = height.saturating_add(1);
                let mut att = chain.attestation_cache.write().await;
                // split_off mutates `att` in place to hold entries < split_key
                // and returns entries >= split_key. We want to keep the latter,
                // so reassign unconditionally — guarding this on `removed > 0`
                // would silently wipe the cache whenever nothing needed pruning.
                let kept = att.split_off(&split_key);
                let removed = att.len();
                *att = kept;
                if removed > 0 {
                    tracing::debug!(
                        chain_key,
                        height,
                        removed,
                        remaining = att.len(),
                        "🔧 🧹 pruned consumed attestations after checkpoint"
                    );
                }
            }
            self.metrics.set_last_attested_height(
                chain_key,
                Self::cached_attestation_height(chain.as_ref()).await,
            );
        }
    }

    /// Insert a checkpoint into the in-memory cache (called from event handler).
    pub async fn insert_checkpoint(&self, chain_key: u64, block_number: u64, digest: H256) {
        if let Some(chain) = self.chains.get(&chain_key) {
            {
                chain
                    .checkpoint_cache
                    .write()
                    .await
                    .insert(block_number, digest);
            }
            tracing::debug!(
                chain_key,
                block_number,
                ?digest,
                "🔧 📦 🚩 checkpoint cached"
            );
            self.metrics.set_last_checkpoint_height(
                chain_key,
                Self::cached_checkpoint_height(chain.as_ref()).await,
            );

            let retention_blocks = self.merkle_cache_retention_blocks(chain.as_ref());
            let min_retained = block_number.saturating_sub(retention_blocks);
            let removed = chain.merkle_proof_cache.prune_below(min_retained).await;
            if removed > 0 {
                tracing::debug!(
                    chain_key,
                    block_number,
                    min_retained,
                    removed,
                    "🔧 🧹 pruned merkle proof cache after checkpoint"
                );
            }
        }
    }

    /// Look up attestation boundaries around a query range from the local cache.
    /// Attestations are more granular than checkpoints, so these provide tighter bounds.
    /// Returns `(lower_block, lower_digest, upper_block, upper_digest)` or `None` if not found.
    ///
    /// The upper digest is the on-chain attested digest at `upper_block`; callers should
    /// verify the proof's computed digest at `upper_block` matches this value so that
    /// the continuity chain is anchored to a known on-chain upper attestation.
    pub async fn get_attestation_boundaries(
        &self,
        chain: &Arc<ChainState>,
        min_query: u64,
        max_query: u64,
    ) -> Option<(u64, H256, u64, H256)> {
        let cache = chain.attestation_cache.read().await;

        // Lower: greatest attestation strictly before min_query.
        let lower = cache.range(..min_query).next_back().map(|(&k, &v)| (k, v));

        // Upper: smallest attestation ≥ max_query
        let upper = cache.range(max_query..).next().map(|(&k, &v)| (k, v));

        match (lower, upper) {
            (Some((lower_block, lower_digest)), Some((upper_block, upper_digest))) => {
                Some((lower_block, lower_digest, upper_block, upper_digest))
            }
            _ => None,
        }
    }

    /// Look up boundaries that combine one half from each cache, used as a
    /// fallback when neither the attestation cache nor the checkpoint cache
    /// brackets the query on its own.
    ///
    /// Both caches store `(block_number, on_chain_digest)` pairs that anchor
    /// the same digest sequence: a checkpoint at block `H` and an attestation
    /// at block `H` reference the exact same on-chain root. The proof builder
    /// only needs one anchor strictly below `min_query` and one anchor at or
    /// above `max_query`; it does not care which cache they come from.
    ///
    /// This function exists to handle the steady state introduced by
    /// [`Self::prune_attestations_at_or_below`]: when a checkpoint at block
    /// `H` lands, all attestations `<= H` are dropped from the attestation
    /// cache. A query for the very first post-checkpoint attestation block
    /// then leaves the attestation cache with only the upper half (no entry
    /// strictly below) and the checkpoint cache with only the lower half (no
    /// entry at or above, since the next checkpoint has not landed yet).
    /// Each cache alone fails to bracket the query, even though the two
    /// halves together describe a perfectly valid anchor pair.
    ///
    /// Tries `(checkpoint_lower, attestation_upper)` first because the
    /// attestation upper is tighter; falls back to
    /// `(attestation_lower, checkpoint_upper)` for the symmetric case.
    /// Returns `(lower_block, lower_digest, upper_block, upper_digest)` or
    /// `None` if the two caches still cannot jointly bracket the query.
    pub async fn get_mixed_boundaries(
        &self,
        chain: &Arc<ChainState>,
        min_query: u64,
        max_query: u64,
    ) -> Option<(u64, H256, u64, H256)> {
        let att = chain.attestation_cache.read().await;
        let cp = chain.checkpoint_cache.read().await;

        // Lower halves: greatest entry strictly before min_query.
        let att_lower = att.range(..min_query).next_back().map(|(&k, &v)| (k, v));
        let cp_lower = cp.range(..min_query).next_back().map(|(&k, &v)| (k, v));

        // Upper halves: smallest entry at or above max_query.
        let att_upper = att.range(max_query..).next().map(|(&k, &v)| (k, v));
        let cp_upper = cp.range(max_query..).next().map(|(&k, &v)| (k, v));

        // Prefer the attestation upper (tighter) paired with the checkpoint lower.
        if let (Some((lo_block, lo_digest)), Some((up_block, up_digest))) = (cp_lower, att_upper) {
            return Some((lo_block, lo_digest, up_block, up_digest));
        }

        // Symmetric fallback: attestation lower with checkpoint upper.
        if let (Some((lo_block, lo_digest)), Some((up_block, up_digest))) = (att_lower, cp_upper) {
            return Some((lo_block, lo_digest, up_block, up_digest));
        }

        None
    }

    /// Look up checkpoint boundaries around a query range from the local cache.
    /// Returns `(lower_block, lower_digest, upper_block, upper_digest)` or `None` if not found.
    ///
    /// The upper digest is the on-chain checkpoint digest at `upper_block`; callers should
    /// verify the proof's computed digest at `upper_block` matches this value so that
    /// the continuity chain is anchored to a known on-chain upper checkpoint.
    pub async fn get_checkpoint_boundaries(
        &self,
        chain: &Arc<ChainState>,
        min_query: u64,
        max_query: u64,
    ) -> Option<(u64, H256, u64, H256)> {
        let cache = chain.checkpoint_cache.read().await;

        // Lower: greatest checkpoint strictly before min_query.
        // Must be strictly less than min_query so that build_proof_from_roots
        // includes the queried block (it builds from lower_checkpoint + 1).
        let lower = cache.range(..min_query).next_back().map(|(&k, &v)| (k, v));

        // Upper: smallest checkpoint ≥ max_query
        let upper = cache.range(max_query..).next().map(|(&k, &v)| (k, v));

        match (lower, upper) {
            (Some((lower_block, lower_digest)), Some((upper_block, upper_digest))) => {
                Some((lower_block, lower_digest, upper_block, upper_digest))
            }
            _ => None,
        }
    }

    /// Operator-oriented detail when boundary resolution failed and we cannot map it to
    /// [`ServiceError::BlockNotReady`] / [`ServiceError::AttestationsMissing`].
    fn format_boundary_miss_ops_detail(
        chain_key: u64,
        min_query: u64,
        max_query: u64,
        att: &BTreeMap<u64, H256>,
        cp: &BTreeMap<u64, H256>,
    ) -> String {
        let att_lower = att.range(..min_query).next_back().map(|(&k, _)| k);
        let att_upper = att.range(max_query..).next().map(|(&k, _)| k);
        let att_span = match (att.keys().next().copied(), att.keys().next_back().copied()) {
            (Some(lo), Some(hi)) => format!("{lo}..={hi}"),
            _ => "empty".to_string(),
        };
        let att_len = att.len();

        let cp_lower = cp.range(..min_query).next_back().map(|(&k, _)| k);
        let cp_upper = cp.range(max_query..).next().map(|(&k, _)| k);
        let cp_span = match (cp.keys().next().copied(), cp.keys().next_back().copied()) {
            (Some(lo), Some(hi)) => format!("{lo}..={hi}"),
            _ => "empty".to_string(),
        };
        let cp_len = cp.len();

        let request_desc = if min_query == max_query {
            format!("requested block {min_query}")
        } else {
            format!("requested blocks {min_query}..={max_query}")
        };

        format!(
            "no attestation or checkpoint window in cache (chain_key={chain_key}) for {request_desc}; \
             need greatest cached block < {min_query} and smallest cached block >= {max_query}. \
             Attestations: n={att_len} span={att_span} greatest_strictly_before={att_lower:?} smallest_at_or_after={att_upper:?}. \
             Checkpoints: n={cp_len} span={cp_span} greatest_strictly_before={cp_lower:?} smallest_at_or_after={cp_upper:?}"
        )
    }

    /// When attestation and checkpoint caches cannot bracket the query, return a client-safe error
    /// when the cause is simply that on-chain data has not caught up yet; otherwise
    /// [`ServiceError::Internal`].
    async fn boundary_lookup_failed_error(
        &self,
        chain: &Arc<ChainState>,
        min_query: u64,
        max_query: u64,
    ) -> ServiceError {
        let chain_key = chain.builder.config.chain_key;
        let att = chain.attestation_cache.read().await;
        let cp = chain.checkpoint_cache.read().await;

        if att.is_empty() && cp.is_empty() {
            return ServiceError::AttestationsMissing { chain_key };
        }

        let last_coverage = match (
            att.keys().next_back().copied(),
            cp.keys().next_back().copied(),
        ) {
            (Some(a), Some(b)) => a.max(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => {
                return ServiceError::AttestationsMissing { chain_key };
            }
        };

        if last_coverage < max_query {
            let ops =
                Self::format_boundary_miss_ops_detail(chain_key, min_query, max_query, &att, &cp);
            tracing::debug!(
                chain_key,
                min_query,
                max_query,
                last_coverage,
                %ops,
                "🔧 ⚠️  boundary miss: returning BlockNotReady to client"
            );
            return ServiceError::BlockNotReady {
                block_number: max_query,
                last_attested_block: last_coverage,
            };
        }

        ServiceError::Internal {
            message: Self::format_boundary_miss_ops_detail(
                chain_key, min_query, max_query, &att, &cp,
            ),
        }
    }

    /// Updates the attestation genesis block number.
    /// This will be called when we receive an AttestationChainGenesisBlockNumberSet event from CC3,
    /// allowing the service to adapt if the attestation genesis block changes (e.g. due to a chain reset or reconfiguration).
    pub async fn update_genesis_block(&self, chain_key: u64, new_genesis_block: u64) {
        if let Some(chain) = self.chains.get(&chain_key) {
            chain
                .attestation_genesis_block
                .store(new_genesis_block, Ordering::Release);

            tracing::info!(
                chain_key,
                attestation_genesis_block = new_genesis_block,
                "🔗 ✅ Updated attestation genesis block"
            );
        }
    }

    /// Build continuity proof for the given blocks, validating them first.
    async fn get_continuity_proof_for(
        &self,
        chain: &Arc<ChainState>,
        headers: &[u64],
    ) -> ServiceResult<ContinuityProof> {
        self.build_continuity(chain, headers)
            .await
            .inspect(|proof| {
                // Record metrics
                self.metrics.observe_proof_blocks(proof.roots.len() as u64);
                // Record timestamp of successful proof generation
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                self.metrics.set_last_proof_generation_timestamp(now);

                tracing::info!(
                    proof_block_count = proof.roots.len(),
                    lower_endpoint_digest = ?proof.lower_endpoint_digest,
                    "🔧 ✨ Generated continuity proof for API response"
                );
            })
    }

    /// Get continuity proof with merkle proof for a transaction at the given index.
    ///
    /// Used by:
    /// - `/api/v1/proof/{chain_key}/{header_number}/{tx_index}`
    /// - `/api/v1/proof-by-tx/{chain_key}/{tx_hash}` (resolves tx_hash to block/index first)
    pub async fn get_proof(
        &self,
        chain: &Arc<ChainState>,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<SingleContinuityResponse> {
        let chain_key = chain.builder.config.chain_key;

        // Validate that the requested block can be processed
        let _current_block = self.validate_blocks(chain, &[header_number]).await?;

        // Record block range metric
        self.metrics.observe_block_range(header_number);

        // Build the continuity proof (a range fetch over many blocks) and the inclusion merkle
        // proof (a single block) concurrently. They touch disjoint RPC and have no data
        // dependency, so overlapping them makes the latency ~max(continuity, merkle) instead of
        // their sum.
        let headers = [header_number];
        let (continuity_proof, merkle) = tokio::try_join!(
            self.get_continuity_proof_for(chain, &headers),
            self.generate_merkle_proof(chain, header_number, tx_index),
        )?;

        let tx_hash = merkle.tx_hash.map(|h| format!("0x{h:x}"));
        let tx_bytes = merkle.tx_bytes.map(|b| format!("0x{}", hex::encode(&b)));

        Ok(SingleContinuityResponse {
            chain_key,
            header_number,
            tx_index,
            tx_hash,
            tx_bytes,
            continuity_proof,
            merkle_proof: merkle.merkle_proof,
            cached: false, // Always false since we generate fresh proofs
            generated_at: Utc::now(),
        })
    }

    async fn get_proof_with_cached_merkle(
        &self,
        chain: &Arc<ChainState>,
        merkle: MerkleProofItem,
    ) -> ServiceResult<SingleContinuityResponse> {
        if merkle.tx_index.is_none() {
            return Err(ServiceError::Internal {
                message: "cached merkle proof missing tx_index".to_string(),
            });
        }
        let header_number = merkle.header_number;
        let chain_key = chain.builder.config.chain_key;

        self.validate_blocks(chain, &[header_number]).await?;
        self.metrics.observe_block_range(header_number);

        let headers = [header_number];
        let continuity_proof = self.get_continuity_proof_for(chain, &headers).await?;

        Ok(Self::single_response_from_merkle(
            chain_key,
            continuity_proof,
            merkle,
            true,
        ))
    }

    fn single_response_from_merkle(
        chain_key: u64,
        continuity_proof: ContinuityProof,
        merkle: MerkleProofItem,
        cached: bool,
    ) -> SingleContinuityResponse {
        let tx_index = merkle.tx_index.unwrap_or_default();
        let tx_hash = merkle.tx_hash.map(|h| format!("0x{h:x}"));
        let tx_bytes = merkle.tx_bytes.map(|b| format!("0x{}", hex::encode(&b)));

        SingleContinuityResponse {
            chain_key,
            header_number: merkle.header_number,
            tx_index,
            tx_hash,
            tx_bytes,
            continuity_proof,
            merkle_proof: merkle.merkle_proof,
            cached,
            generated_at: Utc::now(),
        }
    }

    /// Get batch of proofs for a list of blocks, optionally including merkle proof for specific transactions.
    ///
    /// `queries` is guaranteed to be non-empty and ordered both per header and per transaction index, so we can safely unwrap when building the response.
    ///
    /// Used by:
    /// - `/api/v1/proof-batch/{chain_key}`
    pub async fn get_proof_batch(
        &self,
        chain: &Arc<ChainState>,
        queries: &[ProofQuery],
    ) -> ServiceResult<BatchedContinuityResponse> {
        let chain_key = chain.builder.config.chain_key;

        // Extract unique header numbers for validation and proof building
        let header_numbers = queries
            .iter()
            .map(|q| q.header_number)
            .collect::<Vec<u64>>();
        // Validate that the requested blocks can be processed
        let _current_block = self.validate_blocks(chain, &header_numbers).await?;

        // We can safely unwrap header_numbers here because the API layer guarantees at least one query is present.
        let from_header = *header_numbers.iter().min().unwrap();
        let to_header = *header_numbers.iter().max().unwrap();

        // Enforce maximum batch span to prevent extremely expensive proof
        // generation when a small batch contains widely separated block heights.
        let span = to_header - from_header;
        if span > self.max_batch_span {
            return Err(ServiceError::BatchSpanTooLarge {
                from_block: from_header,
                to_block: to_header,
                span,
                max_span: self.max_batch_span,
            });
        }

        // Record block range metric
        for &header_number in header_numbers.iter() {
            self.metrics.observe_block_range(header_number);
        }

        let mut merkle_proofs = BTreeMap::new();

        // We flatten the list of queries into a list of (header_number, tx_index) pairs to generate merkle proofs for,
        // then regroup them into the desired response structure after generating the proofs.
        let chain_for_merkle = chain.clone();
        let merkle_futures = queries
            .iter()
            .cloned()
            .flat_map(|query| {
                query
                    .tx_indexes
                    .into_iter()
                    .map(move |tx_index| (query.header_number, tx_index))
            })
            .map(move |(header_number, tx_index)| {
                let chain = chain_for_merkle.clone();
                async move {
                    let merkle = self
                        .generate_merkle_proof(&chain, header_number, tx_index)
                        .await?;

                    let entry = BatchedMerkleProofEntry {
                        tx_hash: merkle.tx_hash.map(|h| format!("0x{h:x}")),
                        tx_bytes: merkle.tx_bytes.map(|b| format!("0x{}", hex::encode(&b))),
                        merkle_proof: merkle.merkle_proof,
                    };

                    Ok::<(u64, u64, BatchedMerkleProofEntry), ServiceError>((
                        header_number,
                        tx_index,
                        entry,
                    ))
                }
            });

        use futures::StreamExt as _;
        use futures::TryStreamExt as _;

        // Build the continuity proof (range fetch) concurrently with all per-tx merkle proofs.
        // Disjoint RPC, no data dependency — overlap the continuity range fetch with the merkle
        // batch instead of running them back to back.
        let (continuity_proof, merkle_results) = tokio::try_join!(
            self.get_continuity_proof_for(chain, &header_numbers),
            futures::stream::iter(merkle_futures)
                .buffer_unordered(self.max_batch_size)
                .try_collect::<Vec<_>>(),
        )?;

        for (header_number, tx_index, entry) in merkle_results {
            merkle_proofs
                .entry(header_number)
                .or_insert_with(BTreeMap::new)
                .insert(tx_index, entry);
        }

        Ok(BatchedContinuityResponse {
            chain_key,
            from_header,
            to_header,
            continuity_proof,
            merkle_proofs,
            cached: false, // Always false since we generate fresh proofs
            generated_at: Utc::now(),
        })
    }

    /// Get proof by transaction hash (resolves to block/index, then builds proof).
    /// Used by: `/api/v1/proof-by-tx/{chain_key}/{tx_hash}`
    pub async fn get_proof_by_tx_hash(
        &self,
        chain: &Arc<ChainState>,
        tx_hash: String,
    ) -> ServiceResult<SingleContinuityResponse> {
        let tx_h256 = parse_tx_hash(&tx_hash)?;

        if let Some(merkle) = chain
            .merkle_proof_cache
            .get_by_tx_hash(chain.builder.config.chain_key, tx_h256)
            .await
        {
            tracing::info!(
                chain_key = chain.builder.config.chain_key,
                tx_hash = ?tx_h256,
                header_number = merkle.header_number,
                tx_index = ?merkle.tx_index,
                "merkle proof cache hit by tx hash"
            );
            let response = self.get_proof_with_cached_merkle(chain, merkle).await?;
            return match &response.tx_hash {
                Some(computed_hash) if parse_tx_hash(computed_hash)? == tx_h256 => Ok(response),
                Some(computed_hash) => Err(ServiceError::TxHashNotFound {
                    tx_hash: format!(
                        "Cached transaction hash mismatch: requested 0x{tx_h256:x}, found {computed_hash} at block {} index {}",
                        response.header_number, response.tx_index
                    ),
                }),
                None => Err(ServiceError::Internal {
                    message: format!(
                        "tx_hash missing from cached proof. tx_hash: {tx_h256:x}"
                    ),
                }),
            };
        }

        let (header_number, tx_index) = self
            .get_height_and_index_for_tx_hash(chain, tx_h256)
            .await?;

        let headers = [header_number];
        self.validate_blocks(chain, &headers).await?;
        self.metrics.observe_block_range(header_number);

        let (continuity_proof, on_demand_merkle) = tokio::try_join!(
            self.get_continuity_proof_for(chain, &headers),
            self.precompute_merkle_cache_block_for_tx(chain, header_number, tx_index),
        )?;

        let cached_merkle = if on_demand_merkle.is_some() {
            on_demand_merkle
        } else if let Some(merkle) = chain
            .merkle_proof_cache
            .get_by_tx_hash(chain.builder.config.chain_key, tx_h256)
            .await
        {
            Some(merkle)
        } else {
            chain
                .merkle_proof_cache
                .get_by_block_index(chain.builder.config.chain_key, header_number, tx_index)
                .await
        };

        tracing::info!(
            chain_key = chain.builder.config.chain_key,
            header_number,
            tx_index,
            tx_hash = ?tx_h256,
            cache_hit = cached_merkle.is_some(),
            "resolved proof-by-tx merkle cache after on-demand precompute"
        );

        let response = if let Some(merkle) = cached_merkle {
            Self::single_response_from_merkle(
                chain.builder.config.chain_key,
                continuity_proof,
                merkle,
                true,
            )
        } else {
            let merkle = self
                .generate_merkle_proof(chain, header_number, tx_index)
                .await?;
            Self::single_response_from_merkle(
                chain.builder.config.chain_key,
                continuity_proof,
                merkle,
                false,
            )
        };

        // Verify tx_hash matches
        match &response.tx_hash {
            Some(computed_hash) if parse_tx_hash(computed_hash)? == tx_h256 => Ok(response),
            Some(computed_hash) => Err(ServiceError::TxHashNotFound {
                tx_hash: format!(
                    "Transaction hash mismatch: requested 0x{tx_h256:x}, found {computed_hash} at block {} index {}",
                    response.header_number, response.tx_index
                ),
            }),
            None => Err(ServiceError::Internal {
                message: format!("tx_hash missing from generated proof. tx_hash: {tx_h256:x}"),
            }),
        }
    }

    /// Get proof by transaction hashes (resolves to block/index, then builds proof).
    ///
    /// `tx_hashes` is guaranteed to be non-empty, so we can safely unwrap when building the response.
    ///
    /// Used by: `/api/v1/proof-batch-by-tx/{chain_key}/{tx_hash}`
    pub async fn get_proof_batch_by_tx_hashes(
        &self,
        chain: &Arc<ChainState>,
        tx_hashes: &[String],
    ) -> ServiceResult<BatchedContinuityResponse> {
        // First we parsed all string hashes into H256, returning an error if any are invalid.
        // This avoids doing partial work if there's a bad hash in the list.
        let parsed_hashes = tx_hashes
            .iter()
            .map(|tx_hash| parse_tx_hash(tx_hash))
            .collect::<Result<Vec<H256>, ServiceError>>()?;

        // After that we calculate the block header and transaction index for each hash, returning an error if any hash is not found.
        // Again we do this upfront to avoid doing partial work.
        let header_tx_pairs = {
            let mut futures = Vec::with_capacity(parsed_hashes.len());

            for tx_h256 in parsed_hashes {
                futures.push(self.get_height_and_index_for_tx_hash(chain, tx_h256));
            }

            use futures::StreamExt as _;
            use futures::TryStreamExt as _;

            futures::stream::iter(futures)
                .buffer_unordered(self.max_batch_size)
                .try_collect::<Vec<(u64, u64)>>()
                .await?
        };

        // Now we have all the header numbers and tx indexes, we can build the proofs.
        // We first group by header number to optimize proof building (one proof per block, even if multiple txs), then flatten back out for the response.
        let proof_queries = {
            let mut block_queries = BTreeMap::new();

            for (header_number, tx_index) in header_tx_pairs {
                block_queries
                    .entry(header_number)
                    .or_insert_with(Vec::new)
                    .push(tx_index);
            }

            block_queries
                .into_iter()
                .map(|(header_number, tx_indexes)| ProofQuery {
                    header_number,
                    tx_indexes,
                })
                .collect::<Vec<_>>()
        };

        // Build the continuity proof for all requested blocks and transactions
        self.get_proof_batch(chain, &proof_queries).await
    }
}
