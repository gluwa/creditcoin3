use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use crate::db::{continuity_proofs::ContinuityProofItem, DbManager};
use crate::services::continuity_service::helpers::*;
use attestor_primitives::block::ContinuityProof;
use continuity::{builder::EndsInAttestation, CcRpcProvider, ContinuityBuilder, ContinuityError};
use merkle::proof::TransactionMerkleProof;

pub mod helpers;

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContinuityResponse {
    pub chain_key: u64,
    pub header_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_bytes: Option<String>, // Hex-encoded transaction bytes (payload only)
    pub continuity_proof: ContinuityProof,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merkle_proof: Option<TransactionMerkleProof>,
    pub cached: bool,
    pub generated_at: DateTime<Utc>,
}

use crate::services::errors::ServiceError;
pub type ServiceResult<T> = Result<T, ServiceError>;

/// Sentinel value indicating attestation genesis has not been fetched yet.
/// We use u64::MAX because it's an impossible genesis block number.
const GENESIS_NOT_FETCHED: u64 = u64::MAX;

pub struct ContinuityService {
    builder: Arc<ContinuityBuilder>,
    db: Arc<DbManager>,
    cc3_client: Arc<dyn CcRpcProvider>,
    start_time: Instant,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    /// The genesis block number for the attestation chain.
    /// Blocks before this number cannot be attested to.
    /// Uses GENESIS_NOT_FETCHED as sentinel for "not yet fetched".
    attestation_genesis_block: AtomicU64,
}

impl ContinuityService {
    pub fn new(
        cc3_client: Arc<dyn CcRpcProvider>,
        builder: Arc<ContinuityBuilder>,
        db: Arc<DbManager>,
    ) -> Self {
        Self {
            cc3_client,
            builder,
            db,
            start_time: Instant::now(),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            // Initialize to sentinel, will be fetched lazily on first request
            attestation_genesis_block: AtomicU64::new(GENESIS_NOT_FETCHED),
        }
    }

    /// Get the attestation genesis block number, fetching it if not yet cached.
    async fn get_attestation_genesis(&self) -> ServiceResult<u64> {
        // Check if already cached
        let cached = self.attestation_genesis_block.load(Ordering::Relaxed);
        if cached != GENESIS_NOT_FETCHED {
            return Ok(cached);
        }

        // Fetch from RPC
        let genesis = self
            .builder
            .get_attestation_genesis_block()
            .await
            .map_err(|e| ServiceError::RpcUnavailable {
                message: format!("Failed to get attestation genesis block: {e}"),
            })?;

        // Cache it - use compare_exchange to handle race conditions (first writer wins)
        let _ = self.attestation_genesis_block.compare_exchange(
            GENESIS_NOT_FETCHED,
            genesis,
            Ordering::SeqCst,
            Ordering::Relaxed,
        );

        Ok(genesis)
    }

    /// Validate that the requested block is not before the attestation genesis.
    async fn validate_block_not_before_genesis(&self, header_number: u64) -> ServiceResult<()> {
        let genesis = self.get_attestation_genesis().await?;
        if header_number < genesis {
            tracing::warn!(
                requested_block = header_number,
                genesis_block = genesis,
                "Requested block is before attestation genesis"
            );
            return Err(ServiceError::BlockBeforeGenesis {
                requested_block: header_number,
                genesis_block: genesis,
            });
        }
        Ok(())
    }

    /// Check if block is attested yet. Returns error early if not, avoiding expensive operations.
    /// TODO: replace me when we have attestation event subscriptions implemented.
    async fn validate_block_is_attested(&self, header_number: u64) -> ServiceResult<()> {
        let last_attested_block = self.builder.get_last_attested_block().await.map_err(|e| {
            ServiceError::RpcUnavailable {
                message: format!("Failed to get last attested block: {e}"),
            }
        })?;

        // If None, no blocks have been attested yet - all blocks are "not ready"
        // If Some(n), check if requested block is beyond the last attested
        let is_not_ready = match last_attested_block {
            None => true, // No attestations exist yet
            Some(last) => header_number > last,
        };

        if is_not_ready {
            // For error message: use 0 when no attestations exist to indicate "none attested"
            let last_for_error = last_attested_block.unwrap_or(0);
            tracing::warn!(
                header_number,
                last_attested_block = last_for_error,
                has_attestations = last_attested_block.is_some(),
                "Block not attested yet - failing fast"
            );
            return Err(ServiceError::BlockNotReady {
                block_number: header_number,
                last_attested_block: last_for_error,
            });
        }
        Ok(())
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    pub fn cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }

    pub async fn get_proofs_counts(&self) -> anyhow::Result<(i64, i64, i64)> {
        let bl = self.db.count_block_level().await?;
        // No transaction-level proofs stored separately in the new architecture
        let tl = 0;
        let total = bl;
        Ok((bl, tl, total))
    }

    /// Health check for CC3 RPC connectivity
    pub async fn check_cc3_connectivity(&self) -> anyhow::Result<()> {
        // Try to get the chain name as a basic connectivity check
        let _chain_name = self.builder.get_chain_name().await?;
        Ok(())
    }

    /// Health check for ETH RPC connectivity
    pub async fn check_eth_connectivity(&self) -> anyhow::Result<()> {
        // Try to get the ETH chain ID as a basic connectivity check
        let _chain_id = self.builder.get_eth_chain_id().await?;
        Ok(())
    }

    /// Build and return a continuity proof for a given (chain_key, header_number).
    ///
    /// This method is responsible for:
    /// 1. Performing a cache lookup in the DB (tx_index = NULL case).
    /// 2. Falling back to the underlying `ContinuityBuilder` when no cached entry exists.
    /// 3. Converting the builder-level proof (raw blocks) into the
    ///    production `attestor_primitives::block::ContinuityProof`
    ///    used by the HTTP API.
    /// 4. Persisting newly-built proofs back into the DB.
    ///
    /// How this differs from `build_continuity`:
    /// - `build_continuity` is a small internal helper that always builds a
    ///   fresh proof using the `ContinuityBuilder` and returns an
    ///   in-memory `ContinuityProof` with *no DB reads or writes*.
    ///
    /// - `continuity_proof` is the public service entry point used by the HTTP
    ///   layer. It includes cache lookup, DB insertion, and full response
    ///   construction. This is the method called by the route handlers.
    ///
    /// When to use:
    /// - Use `get_continuity_proof` in all API code paths.
    /// - Use `build_continuity` only inside tests or other internal helpers
    ///   when you explicitly want a fresh proof without touching the DB.
    pub async fn get_continuity_proof(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> ServiceResult<ContinuityResponse> {
        // Validate block is not before attestation genesis
        self.validate_block_not_before_genesis(header_number)
            .await?;

        // Check if block is attested yet (fast check before expensive operations)
        self.validate_block_is_attested(header_number).await?;

        // Attempt to look up continuity proof first
        let maybe_continuity = self
            .fetch_continuity_by_height(chain_key, header_number)
            .await?;

        if let Some(continuity) = maybe_continuity {
            // Check that the continuity proof is verifyable (not based on pruned attestations)
            let verifyable = self.check_continuity_is_current(&continuity).await?;
            if verifyable {
                // Increment cache hit counter
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    chain_key,
                    header_number,
                    "Cache hit: returning cached continuity proof"
                );
                Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: None,
                    tx_hash: None,
                    tx_bytes: None, // continuity proof doesn't include tx bytes
                    continuity_proof: continuity.continuity_proof,
                    merkle_proof: None,
                    cached: true,
                    generated_at: Utc::now(),
                })
            } else {
                // Increment cache miss counter (found but not verifiable)
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    chain_key,
                    header_number,
                    "Cache hit, but continuity proof is no longer verifyable. Rebuilding proof."
                );
                self.build_and_cache_continuity(chain_key, header_number)
                    .await
            }
        } else {
            // Increment cache miss counter (not found)
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
            // Cache miss. Must build continuity.
            self.build_and_cache_continuity(chain_key, header_number)
                .await
        }
    }

    // Top level function responsible for handling requests to the following api endpoint:
    //  /api/v1/proof/{chain_key}/{header_number}/{tx_index}
    pub async fn get_proofs_by_height_and_index(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<ContinuityResponse> {
        // Validate block is not before attestation genesis
        self.validate_block_not_before_genesis(header_number)
            .await?;

        // Check if block is attested yet (fast check before expensive operations)
        self.validate_block_is_attested(header_number).await?;

        // Attempt to fetch continuity proof first
        let maybe_continuity = self
            .fetch_continuity_by_height(chain_key, header_number)
            .await?;

        match maybe_continuity {
            // Case: Continuity present in DB
            Some(continuity) => {
                // Check that the continuity proof is verifyable (not based on pruned attestations)
                let verifyable = self.check_continuity_is_current(&continuity).await?;
                if verifyable {
                    // Increment cache hit counter
                    self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    let merkle = self
                        .generate_merkle_proof(chain_key, header_number, tx_index)
                        .await?;
                    build_response_from_proofs(merkle, continuity)
                } else {
                    // Increment cache miss counter (found but not verifiable)
                    self.cache_misses.fetch_add(1, Ordering::Relaxed);
                    // Continuity present but not verifyable. Must build both proofs
                    self.generate_and_cache_response(chain_key, header_number, tx_index)
                        .await
                }
            }
            None => {
                // Increment cache miss counter (not found)
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
                // Builds both continuity and merkle proofs, then caches continuity proof before returning response
                self.generate_and_cache_response(chain_key, header_number, tx_index)
                    .await
            }
        }
    }

    // Top level function responsible for handling requests to the following api endpoint:
    //  /api/v1/proof-by-tx/{chain_key}/{tx_hash}
    pub async fn get_proofs_by_tx_hash(
        &self,
        chain_key: u64,
        tx_hash: String,
    ) -> ServiceResult<ContinuityResponse> {
        let tx_h256 = parse_tx_hash(&tx_hash)?;

        let (header_number, tx_index) = self.get_height_and_index_for_tx_hash(tx_h256).await?;

        // Validate block is not before attestation genesis
        // (The get_proofs_by_height_and_index call also validates, but we check here for clearer error messages)
        self.validate_block_not_before_genesis(header_number)
            .await?;

        let response = self
            .get_proofs_by_height_and_index(chain_key, header_number, tx_index)
            .await?;

        // Verify that the computed tx_hash matches the requested hash
        if let Some(computed_hash) = &response.tx_hash {
            let computed_h256 = parse_tx_hash(computed_hash)?;
            if computed_h256 != tx_h256 {
                let tx_index = response.tx_index;
                let header_number = response.header_number;
                Err(ServiceError::TxHashNotFound {
                    tx_hash: format!(
                        "Transaction hash mismatch: requested 0x{tx_h256:x}, but found {computed_hash} at block {header_number} index {tx_index:?}"
                    ),
                })
            } else {
                Ok(response)
            }
        } else {
            Err(ServiceError::Internal {
                message: format!(
                    "tx_hash somehow missing from generated proof. tx_hash: {tx_h256:x}"
                ),
            })
        }
    }
}
