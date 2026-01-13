use anyhow::{Context, Result};
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
use continuity::{builder::EndsInAttestation, CcRpcProvider, ContinuityBuilder};
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
#[serde(rename_all = "camelCase")]
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

pub struct ContinuityService {
    builder: Arc<ContinuityBuilder>,
    db: Arc<DbManager>,
    cc3_client: Arc<dyn CcRpcProvider>,
    start_time: Instant,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    /// The genesis block number for the attestation chain.
    /// Blocks before this number cannot be attested to.
    /// Fetched once at service startup and cached for the lifetime of the service.
    attestation_genesis_block: u64,
}

impl ContinuityService {
    /// Create a new ContinuityService, fetching the attestation genesis block from the chain.
    ///
    /// # Errors
    /// Returns an error if the attestation genesis block cannot be fetched from RPC.
    pub async fn new(
        cc3_client: Arc<dyn CcRpcProvider>,
        builder: Arc<ContinuityBuilder>,
        db: Arc<DbManager>,
    ) -> anyhow::Result<Self> {
        // Fetch genesis block at startup - fail fast if RPC is unavailable
        let attestation_genesis_block = builder
            .get_attestation_genesis_block()
            .await
            .context("Failed to fetch attestation genesis block during service initialization")?;

        tracing::info!(
            attestation_genesis_block,
            "ContinuityService initialized with attestation genesis block"
        );

        Ok(Self {
            cc3_client,
            builder,
            db,
            start_time: Instant::now(),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            attestation_genesis_block,
        })
    }

    /// Validate that the requested block can be processed:
    /// 1. Not before attestation genesis
    /// 2. Exists on source chain (ETH)
    ///
    /// Returns the current block height for reuse in validating predicted attestation bounds.
    async fn validate_block(&self, header_number: u64) -> ServiceResult<u64> {
        // Check genesis bound
        if header_number < self.attestation_genesis_block {
            tracing::warn!(
                requested_block = header_number,
                genesis_block = self.attestation_genesis_block,
                "Requested block is before attestation genesis"
            );
            return Err(ServiceError::BlockBeforeGenesis {
                requested_block: header_number,
                genesis_block: self.attestation_genesis_block,
            });
        }

        // Check source chain existence
        let current_block =
            self.builder
                .get_last_block()
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get current block height from source chain: {e}"),
                })?;

        if header_number > current_block {
            tracing::warn!(
                requested_block = header_number,
                current_block,
                "Requested block does not exist on source chain yet"
            );
            return Err(ServiceError::BlockNotOnSourceChain {
                requested_block: header_number,
                current_block,
            });
        }
        Ok(current_block)
    }

    /// Try to get a valid cached continuity proof, updating cache counters.
    /// Returns `Some(proof)` on cache hit, `None` on cache miss or invalid cache.
    async fn try_get_cached_continuity(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> ServiceResult<Option<ContinuityProofItem>> {
        let maybe_continuity = self
            .fetch_continuity_by_height(chain_key, header_number)
            .await?;

        match maybe_continuity {
            Some(continuity) => {
                let verifiable = self.check_continuity_is_current(&continuity).await?;
                if verifiable {
                    self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(
                        chain_key,
                        header_number,
                        "Cache hit: returning cached continuity proof"
                    );
                    Ok(Some(continuity))
                } else {
                    self.cache_misses.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(
                        chain_key,
                        header_number,
                        "Cache hit, but continuity proof is no longer verifiable. Rebuilding."
                    );
                    Ok(None)
                }
            }
            None => {
                self.cache_misses.fetch_add(1, Ordering::Relaxed);
                Ok(None)
            }
        }
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

    /// Get proof for a block, optionally including merkle proof for a specific transaction.
    /// - `tx_index = None`: returns continuity proof only
    /// - `tx_index = Some(idx)`: returns continuity + merkle proof for transaction at `idx`
    ///
    /// Used by:
    /// - `/api/v1/proof/{chain_key}/{header_number}` (tx_index = None)
    /// - `/api/v1/proof/{chain_key}/{header_number}/{tx_index}` (tx_index = Some)
    pub async fn get_proof(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: Option<u64>,
    ) -> ServiceResult<ContinuityResponse> {
        let current_block = self.validate_block(header_number).await?;

        if let Some(continuity) = self
            .try_get_cached_continuity(chain_key, header_number)
            .await?
        {
            match tx_index {
                Some(idx) => {
                    let merkle = self
                        .generate_merkle_proof(chain_key, header_number, idx)
                        .await?;
                    build_response_from_proofs(merkle, continuity)
                }
                None => Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: None,
                    tx_hash: None,
                    tx_bytes: None,
                    continuity_proof: continuity.continuity_proof,
                    merkle_proof: None,
                    cached: true,
                    generated_at: Utc::now(),
                }),
            }
        } else {
            self.generate_and_cache_response(chain_key, header_number, tx_index, current_block)
                .await
        }
    }

    /// Get proofs by transaction hash (resolves to block/index, then builds proofs).
    /// Used by: `/api/v1/proof-by-tx/{chain_key}/{tx_hash}`
    pub async fn get_proofs_by_tx_hash(
        &self,
        chain_key: u64,
        tx_hash: String,
    ) -> ServiceResult<ContinuityResponse> {
        let tx_h256 = parse_tx_hash(&tx_hash)?;
        let (header_number, tx_index) = self.get_height_and_index_for_tx_hash(tx_h256).await?;

        let response = self
            .get_proof(chain_key, header_number, Some(tx_index))
            .await?;

        // Verify tx_hash matches
        match &response.tx_hash {
            Some(computed_hash) if parse_tx_hash(computed_hash)? == tx_h256 => Ok(response),
            Some(computed_hash) => Err(ServiceError::TxHashNotFound {
                tx_hash: format!(
                    "Transaction hash mismatch: requested 0x{tx_h256:x}, found {computed_hash} at block {} index {:?}",
                    response.header_number, response.tx_index
                ),
            }),
            None => Err(ServiceError::Internal {
                message: format!("tx_hash missing from generated proof. tx_hash: {tx_h256:x}"),
            }),
        }
    }
}
