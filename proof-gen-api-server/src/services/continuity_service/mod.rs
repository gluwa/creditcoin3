use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use hex;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

use crate::services::continuity_service::helpers::*;
use attestor_primitives::block::ContinuityProof as AttestorContinuityProof;
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
    pub continuity_proof: AttestorContinuityProof,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merkle_proof: Option<TransactionMerkleProof>,
    pub cached: bool,
    pub generated_at: DateTime<Utc>,
}

use crate::services::errors::ServiceError;
pub type ServiceResult<T> = Result<T, ServiceError>;

pub struct ContinuityService {
    builder: Arc<ContinuityBuilder>,
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
        // No database to count from - return cache statistics instead
        let hits = self.cache_hits.load(Ordering::Relaxed) as i64;
        let misses = self.cache_misses.load(Ordering::Relaxed) as i64;
        let total = hits + misses;
        Ok((hits, misses, total))
    }

    /// Health check for CC3 RPC connectivity
    pub async fn check_cc3_connectivity(&self) -> anyhow::Result<()> {
        // Try to get the chain name as a basic connectivity check
        let _chain_name = self.builder.get_chain_name().await?;
        // Note: cc3_client is kept for potential future use (e.g., checkpoint checks)
        let _ = &self.cc3_client;
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

        // ContinuityBuilder will automatically use indexer if available
        // Track cache hits/misses based on whether indexer was used
        let (continuity_proof, was_cached) =
            match self.build_continuity(header_number, current_block).await {
                Ok((proof, _ends_in_attestation, lower_attestation)) => {
                    // Note: ContinuityBuilder handles indexer internally, so we can't easily detect
                    // if indexer was used. For now, we'll always mark as not cached since we're
                    // building fresh proofs (even if they use indexer data internally).
                    let cached = false;
                    if cached {
                        self.cache_hits.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.cache_misses.fetch_add(1, Ordering::Relaxed);
                    }

                    // Convert BuiltContinuityProof to attestor_primitives::ContinuityProof
                    // Uses smart conversion that handles trimmed proofs and attestation context
                    let attestor_proof = proof
                        .to_attestor_proof_with_attestation_context(lower_attestation.digest)
                        .unwrap_or_default();

                    tracing::info!(
                        proof_block_count = proof.blocks.len(),
                        lower_endpoint_digest = ?attestor_proof.lower_endpoint_digest,
                        first_block_number = proof.blocks.first().map(|b| b.block_number),
                        last_block_number = proof.blocks.last().map(|b| b.block_number),
                        last_block_digest = ?proof.blocks.last().map(|b| b.digest),
                        "Converting continuity proof for API response"
                    );
                    (attestor_proof, cached)
                }
                Err(e) => return Err(e),
            };

        match tx_index {
            Some(idx) => {
                let merkle = self
                    .generate_merkle_proof(chain_key, header_number, idx)
                    .await?;
                Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: Some(idx),
                    tx_hash: merkle.tx_hash.map(|h| format!("0x{h:x}")),
                    tx_bytes: merkle.tx_bytes.map(|b| format!("0x{}", hex::encode(&b))),
                    continuity_proof,
                    merkle_proof: Some(merkle.merkle_proof),
                    cached: was_cached,
                    generated_at: Utc::now(),
                })
            }
            None => Ok(ContinuityResponse {
                chain_key,
                header_number,
                tx_index: None,
                tx_hash: None,
                tx_bytes: None,
                continuity_proof,
                merkle_proof: None,
                cached: was_cached,
                generated_at: Utc::now(),
            }),
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
