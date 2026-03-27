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
use std::time::Instant;

use crate::prom::Metrics;
use crate::services::continuity_service::helpers::*;
use attestor_primitives::block::ContinuityProof;
use continuity::ContinuityBuilder;
use merkle::proof::TransactionMerkleProof;

pub mod helpers;

/// Per-source-chain state: ETH-backed builder and CC3-derived caches.
pub struct ChainState {
    pub builder: Arc<ContinuityBuilder>,
    pub checkpoint_cache: tokio::sync::RwLock<BTreeMap<u64, H256>>,
    pub attestation_cache: tokio::sync::RwLock<BTreeMap<u64, H256>>,
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
    /// Total number of proof requests processed (for health endpoint statistics)
    total_proof_requests: AtomicU64,
    /// Prometheus metrics for instrumentation (uses NoopMetrics when disabled).
    metrics: Metrics,
    /// Maximum amount of concurrent futures spawned when generating proofs for batch requests or when extracting transaction indexes from transaction hashes.
    max_batch_size: usize,
}

impl ContinuityService {
    /// Create a new ContinuityService, fetching the attestation genesis block from the chain.
    ///
    /// # Errors
    /// Returns an error if the attestation genesis block cannot be fetched from RPC.
    pub async fn new(
        builders: Vec<Arc<ContinuityBuilder>>,
        metrics: Metrics,
        max_batch_size: usize,
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
                "[startup] ContinuityService: fetching attestation genesis block from CC3"
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
                "ContinuityService chain initialized with attestation genesis block"
            );

            // Populate checkpoint cache from CC3 on startup.
            tracing::debug!(
                chain_key,
                "⏳ Populating checkpoint cache from CC3 (this may take a while)..."
            );
            let checkpoints = builder
                .cc_provider
                .get_checkpoints_for_chain(chain_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        chain_key,
                        "Failed to fetch checkpoints on startup: {e}, starting with empty cache"
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
                "Checkpoint cache populated from CC3"
            );

            // Populate attestation cache from CC3 on startup.
            tracing::debug!(
                chain_key,
                "⏳ Populating attestation cache from CC3 (this may take a while)..."
            );
            let attestations = builder
                .cc_provider
                .get_attestations_for_chain(chain_key)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        chain_key,
                        "Failed to fetch attestations on startup: {e}, starting with empty cache"
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
                "Attestation cache populated from CC3"
            );

            chains.insert(
                chain_key,
                Arc::new(ChainState {
                    builder,
                    checkpoint_cache: tokio::sync::RwLock::new(checkpoint_map),
                    attestation_cache: tokio::sync::RwLock::new(attestation_map),
                    attestation_genesis_block: AtomicU64::new(attestation_genesis_block),
                }),
            );
        }

        Ok(Self {
            chains,
            start_time: Instant::now(),
            total_proof_requests: AtomicU64::new(0),
            metrics,
            max_batch_size,
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
                "Requested block is before or at attestation genesis"
            );
            return Err(ServiceError::BlockBeforeOrAtGenesis {
                requested_block: header_number,
                genesis_block,
            });
        }

        // Check source chain existence
        let current_block =
            chain
                .builder
                .get_last_block()
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get current block height from source chain: {e}"),
                })?;

        if let Some(&header_number) = header_numbers.iter().find(|h| **h > current_block) {
            tracing::warn!(
                requested_block = header_number,
                current_block,
                chain_key,
                "Requested block does not exist on source chain yet"
            );
            return Err(ServiceError::BlockNotOnSourceChain {
                requested_block: header_number,
                current_block,
            });
        }

        Ok(current_block)
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Get total number of proof requests processed.
    /// Returns (total_requests) for use in health endpoint.
    pub async fn get_proofs_counts(&self) -> anyhow::Result<i64> {
        // Return total proof requests processed since service start
        let total = self.total_proof_requests.load(Ordering::Relaxed) as i64;
        Ok(total)
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
                .get_eth_chain_id()
                .await
                .map_err(|e| anyhow::anyhow!("eth RPC chain {chain_key}: {e}"))?;
        }
        Ok(())
    }

    /// Insert an attestation into the in-memory cache (called from event handler).
    pub async fn insert_attestation(&self, chain_key: u64, block_number: u64, digest: H256) {
        if let Some(chain) = self.chains.get(&chain_key) {
            chain
                .attestation_cache
                .write()
                .await
                .insert(block_number, digest);
            tracing::debug!(chain_key, block_number, ?digest, "attestation cached");
        }
    }

    /// Insert a checkpoint into the in-memory cache (called from event handler).
    pub async fn insert_checkpoint(&self, chain_key: u64, block_number: u64, digest: H256) {
        if let Some(chain) = self.chains.get(&chain_key) {
            chain
                .checkpoint_cache
                .write()
                .await
                .insert(block_number, digest);
            tracing::debug!(chain_key, block_number, ?digest, "checkpoint cached");
        }
    }

    /// Look up attestation boundaries around a query range from the local cache.
    /// Attestations are more granular than checkpoints, so these provide tighter bounds.
    /// Returns `(lower_block, lower_digest, upper_block)` or `None` if not found.
    pub async fn get_attestation_boundaries(
        &self,
        chain: &Arc<ChainState>,
        min_query: u64,
        max_query: u64,
    ) -> Option<(u64, H256, u64)> {
        let cache = chain.attestation_cache.read().await;

        // Lower: greatest attestation strictly before min_query.
        let lower = cache.range(..min_query).next_back().map(|(&k, &v)| (k, v));

        // Upper: smallest attestation ≥ max_query
        let upper = cache.range(max_query..).next().map(|(&k, _)| k);

        match (lower, upper) {
            (Some((lower_block, lower_digest)), Some(upper_block)) => {
                Some((lower_block, lower_digest, upper_block))
            }
            _ => None,
        }
    }

    /// Look up checkpoint boundaries around a query range from the local cache.
    /// Returns `(lower_block, lower_digest, upper_block)` or `None` if not found.
    pub async fn get_checkpoint_boundaries(
        &self,
        chain: &Arc<ChainState>,
        min_query: u64,
        max_query: u64,
    ) -> Option<(u64, H256, u64)> {
        let cache = chain.checkpoint_cache.read().await;

        // Lower: greatest checkpoint strictly before min_query.
        // Must be strictly less than min_query so that build_proof_from_roots
        // includes the queried block (it builds from lower_checkpoint + 1).
        let lower = cache.range(..min_query).next_back().map(|(&k, &v)| (k, v));

        // Upper: smallest checkpoint ≥ max_query
        let upper = cache.range(max_query..).next().map(|(&k, _)| k);

        match (lower, upper) {
            (Some((lower_block, lower_digest)), Some(upper_block)) => {
                Some((lower_block, lower_digest, upper_block))
            }
            _ => None,
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
                "Updated attestation genesis block"
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
                // Increment total proof requests counter
                self.total_proof_requests.fetch_add(1, Ordering::Relaxed);

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
                    "Generated continuity proof for API response"
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

        let continuity_proof = self
            .get_continuity_proof_for(chain, &[header_number])
            .await?;

        let merkle = self
            .generate_merkle_proof(chain, header_number, tx_index)
            .await?;

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

        // Record block range metric
        for &header_number in header_numbers.iter() {
            self.metrics.observe_block_range(header_number);
        }

        let continuity_proof = self
            .get_continuity_proof_for(chain, &header_numbers)
            .await?;

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

        for (header_number, tx_index, entry) in futures::stream::iter(merkle_futures)
            .buffer_unordered(self.max_batch_size)
            .try_collect::<Vec<_>>()
            .await?
        {
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
        let (header_number, tx_index) = self
            .get_height_and_index_for_tx_hash(chain, tx_h256)
            .await?;

        let response = self.get_proof(chain, header_number, tx_index).await?;

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
