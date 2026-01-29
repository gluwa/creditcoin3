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

use crate::prom::Metrics;
use crate::services::continuity_service::helpers::*;
use attestor_primitives::block::ContinuityProof;
use continuity::ContinuityBuilder;
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
    start_time: Instant,
    /// Total number of proof requests processed (for health endpoint statistics)
    total_proof_requests: AtomicU64,
    /// The genesis block number for the attestation chain.
    /// Blocks before this number cannot be attested to.
    /// Fetched once at service startup and cached for the lifetime of the service.
    attestation_genesis_block: u64,
    /// Optional prometheus metrics for instrumentation.
    metrics: Option<Metrics>,
}

impl ContinuityService {
    /// Create a new ContinuityService, fetching the attestation genesis block from the chain.
    ///
    /// # Errors
    /// Returns an error if the attestation genesis block cannot be fetched from RPC.
    pub async fn new(
        builder: Arc<ContinuityBuilder>,
        metrics: Option<Metrics>,
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
            builder,
            start_time: Instant::now(),
            total_proof_requests: AtomicU64::new(0),
            attestation_genesis_block,
            metrics,
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

    /// Get total number of proof requests processed.
    /// Returns (total_requests) for use in health endpoint.
    pub async fn get_proofs_counts(&self) -> anyhow::Result<i64> {
        // Return total proof requests processed since service start
        let total = self.total_proof_requests.load(Ordering::Relaxed) as i64;
        Ok(total)
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

        // Record block range metric
        if let Some(ref m) = self.metrics {
            m.observe_block_range(header_number);
        }

        // ContinuityBuilder will automatically use indexer if available
        let (continuity_proof, was_cached) =
            match self.build_continuity(header_number, current_block).await {
                Ok((proof, _lower_attestation)) => {
                    // Increment total proof requests counter
                    self.total_proof_requests.fetch_add(1, Ordering::Relaxed);

                    // Record metrics
                    if let Some(ref m) = self.metrics {
                        m.observe_proof_blocks(proof.roots.len() as u64);
                        // Record timestamp of successful proof generation
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs_f64())
                            .unwrap_or(0.0);
                        m.set_last_proof_generation_timestamp(now);
                    }

                    tracing::info!(
                        proof_block_count = proof.roots.len(),
                        lower_endpoint_digest = ?proof.lower_endpoint_digest,
                        "Generated continuity proof for API response"
                    );
                    (proof, false) // Always false since we generate fresh proofs
                }
                Err(e) => {
                    return Err(e);
                }
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
