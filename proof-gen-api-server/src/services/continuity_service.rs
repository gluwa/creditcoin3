use crate::db::{DbManager, QueryProofs};
use attestor_primitives::block::ContinuityProof;
use attestor_primitives::Query;
use chrono::{DateTime, Utc};
use continuity::{ContinuityBuilder, ContinuityProof as RawContinuityProof};
use mmr::query_proof::QueryMerkleProof;
use serde::{Deserialize, Serialize};
use sp_core::hashing::keccak_256;
use sp_core::H256;
use std::sync::Arc;

// === Serialization helpers ===
// Remove helper; use LowerHex formatting on H256 directly where needed.

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContinuityResponse {
    pub chain_key: u64,
    pub header_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    pub continuity_proof: ContinuityProof,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merkle_proof: Option<QueryMerkleProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merkle_root: Option<String>,
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

    /// Internal helper that always builds a fresh continuity proof directly
    /// using the underlying `ContinuityBuilder`.
    ///
    /// Does *not*:
    /// - perform DB cache lookups
    /// - write results to the DB
    /// - construct an HTTP-facing response type
    ///
    /// This is mainly useful inside tests or internal utilities that need a
    /// raw proof without involving the persistence layer.
    async fn build_continuity(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> Result<ContinuityProof, ServiceError> {
        let query = Query {
            chain_id: chain_key,
            height: header_number,
            layout_segments: vec![],
        };
        let proof: RawContinuityProof = self
            .builder
            .build_for_single_query(&query)
            .await
            .map_err(ServiceError::from)?;
        Ok(ContinuityProof::from_blocks(proof.blocks.clone()))
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
    /// - Use `continuity_proof` in all API code paths.
    /// - Use `build_continuity` only inside tests or other internal helpers
    ///   when you explicitly want a fresh proof without touching the DB.
    pub async fn continuity_proof(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> ServiceResult<ContinuityResponse> {
        // Attempt cache lookup using block-level retrieval (tx_index IS NULL)
        match self.db.get_proofs_for_block(chain_key, header_number).await {
            Ok(Some(entry)) => {
                let proofs = QueryProofs::try_from(entry).map_err(|e| ServiceError::DbError {
                    message: e.to_string(),
                })?;
                if let Some(continuity_proof_out) = proofs.continuity_proof {
                    return Ok(ContinuityResponse {
                        chain_key,
                        header_number,
                        tx_index: None,
                        tx_hash: None,
                        continuity_proof: continuity_proof_out,
                        merkle_proof: None,
                        merkle_root: proofs.merkle_root.map(|h| format!("0x{h:x}")),
                        cached: true,
                        generated_at: Utc::now(),
                    });
                }
            }
            Ok(None) => { /* cache miss, build new */ }
            Err(e) => {
                tracing::warn!(error=?e, chain_key, header_number, "DB error during continuity_proof cache lookup; proceeding to build new proof");
            }
        }

        // Build new continuity proof
        let query = Query {
            chain_id: chain_key,
            height: header_number,
            layout_segments: vec![],
        };
        let proof: RawContinuityProof = self
            .builder
            .build_for_single_query(&query)
            .await
            .map_err(ServiceError::from)?;
        // Convert raw blocks into optimized ContinuityProof (attestor primitives)
        let continuity_out = ContinuityProof::from_blocks(proof.blocks.clone());

        // Insert into DB asynchronously in background
        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: None,
            tx_hash: None,
            continuity_proof: Some(continuity_out.clone()),
            merkle_proof: None,
            merkle_root: None,
        };
        self.db.insert_proofs_entry(entry);

        Ok(ContinuityResponse {
            chain_key,
            header_number,
            tx_index: None,
            tx_hash: None,
            continuity_proof: continuity_out,
            merkle_proof: None,
            merkle_root: None,
            cached: false,
            generated_at: Utc::now(),
        })
    }

    pub async fn continuity_proof_with_tx(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<ContinuityResponse> {
        // 1. Attempt tx-specific cache hit.
        if let Ok(Some(entry)) = self
            .db
            .get_proofs_for_tx(chain_key, header_number, tx_index)
            .await
        {
            let proofs = QueryProofs::try_from(entry).map_err(|e| ServiceError::DbError {
                message: e.to_string(),
            })?;
            if let (Some(continuity_out), Some(merkle_proof)) =
                (proofs.continuity_proof, proofs.merkle_proof)
            {
                return Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: Some(tx_index),
                    // Propagate cached tx_hash (was previously None causing tests to fail)
                    tx_hash: proofs.tx_hash.map(|h| format!("0x{h:x}")),
                    continuity_proof: continuity_out,
                    merkle_proof: Some(merkle_proof.clone()),
                    merkle_root: proofs.merkle_root.map(|h| format!("0x{h:x}")),
                    cached: true,
                    generated_at: Utc::now(),
                });
            }
        }

        // 2. Try block-level continuity cache.
        let continuity_out = if let Ok(Some(block_entry)) =
            self.db.get_proofs_for_block(chain_key, header_number).await
        {
            let proofs = QueryProofs::try_from(block_entry).map_err(|e| ServiceError::DbError {
                message: e.to_string(),
            })?;
            if let Some(cp) = proofs.continuity_proof {
                cp
            } else {
                self.build_continuity(chain_key, header_number).await?
            }
        } else {
            self.build_continuity(chain_key, header_number).await?
        };

        // 3. Fetch tx bytes & validate index.
        let tx_bytes = self
            .builder
            .get_block_tx_bytes(header_number)
            .await
            .map_err(|e| ServiceError::RpcUnavailable {
                message: e.to_string(),
            })?;
        if tx_bytes.is_empty() {
            if tx_index != 0 {
                return Err(ServiceError::TxIndexOutOfBounds { tx_index, len: 0 });
            }
        } else if tx_index as usize >= tx_bytes.len() {
            return Err(ServiceError::TxIndexOutOfBounds {
                tx_index,
                len: tx_bytes.len(),
            });
        }

        // 4. Merkle proof creation and tx hash computation.
        let tree = mmr::SimpleMerkleTree::new(&tx_bytes);
        let merkle_proof = if tx_bytes.is_empty() {
            QueryMerkleProof::new(tree.root(), vec![])
        } else {
            tree.generate_proof(tx_index as usize)
        };
        let merkle_root = tree.root();

        // Compute tx_hash if there is at least one transaction
        let tx_hash_opt = if tx_bytes.is_empty() {
            None
        } else {
            let bytes = &tx_bytes[tx_index as usize];
            Some(H256::from(keccak_256(bytes)))
        };

        // 5. Insert tx-specific entry (fail fast if unavailable).
        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: tx_hash_opt,
            continuity_proof: Some(continuity_out.clone()),
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(merkle_root),
        };
        // Insert into DB asynchronously in background
        self.db.insert_proofs_entry(entry);

        // 6. Return response (cached=false because tx-specific entry was missing).
        Ok(ContinuityResponse {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: tx_hash_opt.map(|h| format!("0x{h:x}")),
            continuity_proof: continuity_out,
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(format!("0x{merkle_root:x}")),
            cached: false,
            generated_at: Utc::now(),
        })
    }

    fn parse_tx_hash(&self, tx_hash: &str) -> Result<H256, ServiceError> {
        let clean = tx_hash.trim_start_matches("0x");
        let bytes = hex::decode(clean).map_err(|e| ServiceError::InvalidParameter {
            message: format!("invalid tx_hash hex: {e}"),
        })?;
        if bytes.len() != 32 {
            let len = bytes.len();
            return Err(ServiceError::InvalidParameter {
                message: format!("tx_hash must be 32 bytes, got {len}"),
            });
        }
        Ok(H256::from_slice(&bytes))
    }

    pub async fn continuity_proof_by_tx_hash(
        &self,
        chain_key: u64,
        tx_hash: String,
    ) -> ServiceResult<ContinuityResponse> {
        // 0. Parse tx_hash
        let tx_h256 = self.parse_tx_hash(&tx_hash)?;

        // 1. Try DB lookup by tx_hash
        match self.db.get_proofs_by_tx_hash(chain_key, tx_h256).await {
            Ok(Some(entry)) => {
                let header_number_u64 = entry.header_number as u64;
                let tx_index_opt = entry.tx_index;

                // Deserialize cached proofs using type_conversions.
                let proofs = QueryProofs::try_from(entry).map_err(|e| {
                    tracing::error!(%tx_hash, chain_key, error=%e, "Cached proofs deserialization failed");
                    ServiceError::DbError {
                        message: e.to_string(),
                    }
                })?;

                if let (Some(continuity_out), Some(merkle_proof)) =
                    (proofs.continuity_proof.clone(), proofs.merkle_proof.clone())
                {
                    tracing::debug!(%tx_hash, chain_key, header_number=header_number_u64, "Cache hit: returning cached proofs");
                    return Ok(ContinuityResponse {
                        chain_key,
                        header_number: header_number_u64,
                        tx_index: proofs.tx_index,
                        tx_hash: Some(tx_hash),
                        continuity_proof: continuity_out,
                        merkle_proof: Some(merkle_proof.clone()),
                        merkle_root: proofs.merkle_root.map(|h| format!("0x{h:x}")),
                        cached: true,
                        generated_at: Utc::now(),
                    });
                }

                // Rebuild path: either missing continuity or merkle proof
                if let Some(tx_index_u64) = tx_index_opt.map(|i| i as u64) {
                    tracing::debug!(%tx_hash, chain_key, header_number=header_number_u64, "Partial cache entry; rebuilding proofs");
                    let rebuilt = self
                        .continuity_proof_with_tx(chain_key, header_number_u64, tx_index_u64)
                        .await?;
                    return Ok(ContinuityResponse {
                        cached: false,
                        ..rebuilt
                    });
                }

                // Neither full proofs nor tx index: treat as not found
                tracing::warn!(%tx_hash, chain_key, header_number=header_number_u64, "Cache entry missing required data for rebuild");
                Err(ServiceError::TxHashNotFound { tx_hash })
            }
            Ok(None) => {
                // DB miss: attempt RPC resolution and generate proofs
                match self.builder.get_tx_position_by_hash(tx_h256).await {
                    Ok((header_number, tx_index)) => {
                        let generated = self
                            .continuity_proof_with_tx(chain_key, header_number, tx_index)
                            .await?;
                        Ok(ContinuityResponse {
                            cached: false,
                            ..generated
                        })
                    }
                    Err(e) => Err(ServiceError::RpcUnavailable {
                        message: format!("failed to resolve tx by hash via RPC: {e}"),
                    }),
                }
            }
            Err(e) => Err(ServiceError::DbError {
                message: e.to_string(),
            }),
        }
    }
}
