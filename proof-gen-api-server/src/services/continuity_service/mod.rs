use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::sync::Arc;

use crate::db::{
    continuity_proofs::ContinuityProofItem, DbManager,
};
use crate::services::continuity_service::helpers::*;
use attestor_primitives::block::ContinuityProof;
use continuity::{ContinuityBuilder, ContinuityError, ContinuityProof as RawContinuityProof};
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
    pub tx_bytes: Option<Vec<u8>>, // Cached transaction bytes (includes BlockItem identifier prefix)
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
    pub tx_bytes: Option<String>, // Hex-encoded transaction bytes (includes BlockItem identifier prefix)
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
}

impl ContinuityService {
    pub fn new(builder: Arc<ContinuityBuilder>, db: Arc<DbManager>) -> Self {
        Self { builder, db }
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
        let current_block =
            self.builder
                .get_last_block()
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get current block height: {e}"),
                })?;

        // Attempt to look up continuity proof first
        let maybe_continuity = self
            .db
            .get_continuity_proof_for_block(chain_key, header_number)
            .await
            .map_err(|e| ServiceError::DbError {
                message: e.to_string(),
            })?;
        let converted_continuity = maybe_continuity
            .map(ContinuityProofItem::try_from)
            .transpose()
            .map_err(|e| ServiceError::DbError {
                message: e.to_string(),
            })?;

        if let Some(continuity) = converted_continuity {
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
            tracing::info!(
                chain_key,
                header_number,
                "Building continuity proof (cache miss)"
            );
            let proof: RawContinuityProof = self
                .builder
                .build_for_single_query(header_number)
                .await
                .map_err(|e| self.handle_build_error(e, header_number, current_block))?;
            // Convert raw blocks into optimized ContinuityProof (attestor primitives)
            let continuity = ContinuityProof::from_blocks(proof.blocks.clone());
            tracing::info!(
                chain_key,
                header_number,
                blocks = continuity.blocks.len(),
                "Continuity proof built successfully"
            );

            // Insert into DB asynchronously in background
            let entry = ContinuityProofItem {
                chain_key,
                header_number,
                continuity_proof: continuity.clone(),
            };
            self.db.insert_continuity_proof(entry);

            Ok(ContinuityResponse {
                chain_key,
                header_number,
                tx_index: None,
                tx_hash: None,
                tx_bytes: None, // Block-level proof doesn't include tx bytes
                continuity_proof: continuity,
                merkle_proof: None,
                cached: false,
                generated_at: Utc::now(),
            })
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
        // Attempt to fetch both proofs from their respective tables
        let continuity = match self
            .fetch_continuity_by_height(chain_key, header_number)
            .await
        {
            Ok(proofs) => proofs,
            Err(e) => {
                tracing::error!(chain_key, header_number, tx_index, error=%e, "Failed to fetch db proofs by header_number and tx_index");
                return Err(ServiceError::DbError {
                    message: e.to_string(),
                });
            }
        };

        match continuity {
            // Case: Continuity present in DB
            Some(continuity) => {
                let merkle = self.generate_merkle_proof(chain_key, header_number, tx_index).await?;
                build_response_from_proofs(merkle, continuity)
            }
            None => {
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

        let response = self.get_proofs_by_height_and_index(chain_key, header_number, tx_index).await?;

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
