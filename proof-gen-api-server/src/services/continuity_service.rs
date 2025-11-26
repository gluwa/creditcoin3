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
fn h256_to_hex(h: &H256) -> String {
    // still used for merkle proof output wrapping
    let hex_bytes = hex::encode(h.as_bytes());
    format!("0x{hex_bytes}")
}

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

pub struct ContinuityService {
    builder: Arc<ContinuityBuilder>,
    db: Arc<DbManager>,
}

impl ContinuityService {
    pub fn new(builder: Arc<ContinuityBuilder>, db: Arc<DbManager>) -> Self {
        Self { builder, db }
    }

    // Helper: build continuity proof for a single (chain_key, header_number) query
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

    pub async fn continuity_proof(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> std::result::Result<ContinuityResponse, ServiceError> {
        // Attempt cache lookup using block-level retrieval (tx_index IS NULL)
        match self.db.get_proofs_for_block(chain_key, header_number).await {
            Ok(Some(entry)) => {
                if let Some(cp_json) = entry.continuity_proof.clone() {
                    if let Ok(continuity_proof_out) =
                        serde_json::from_value::<ContinuityProof>(cp_json)
                    {
                        return Ok(ContinuityResponse {
                            chain_key,
                            header_number,
                            tx_index: None,
                            tx_hash: None,
                            continuity_proof: continuity_proof_out,
                            merkle_proof: None,
                            merkle_root: entry.merkle_root.clone(),
                            cached: true,
                            generated_at: Utc::now(),
                        });
                    }
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

        // Insert into DB (fail fast if unavailable)
        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: None,
            tx_hash: None,
            continuity_proof: Some(continuity_out.clone()),
            merkle_proof: None,
            merkle_root: None,
        };
        if let Err(e) = self.db.try_insert_proofs_entry(entry).await {
            return Err(ServiceError::DbError {
                message: e.to_string(),
            });
        }

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
    ) -> std::result::Result<ContinuityResponse, ServiceError> {
        // 1. Attempt tx-specific cache hit.
        if let Ok(Some(entry)) = self
            .db
            .get_proofs_for_tx(chain_key, header_number, tx_index)
            .await
        {
            let continuity_opt = entry
                .continuity_proof
                .clone()
                .and_then(|v| serde_json::from_value::<ContinuityProof>(v).ok());
            let merkle_opt = entry
                .merkle_proof
                .clone()
                .and_then(|v| serde_json::from_value::<QueryMerkleProof>(v).ok());
            if let (Some(continuity_out), Some(merkle_proof)) = (continuity_opt, merkle_opt) {
                return Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: Some(tx_index),
                    // Propagate cached tx_hash (was previously None causing tests to fail)
                    tx_hash: entry.tx_hash.clone(),
                    continuity_proof: continuity_out,
                    merkle_proof: Some(merkle_proof.clone()),
                    merkle_root: entry.merkle_root.clone(),
                    cached: true,
                    generated_at: Utc::now(),
                });
            }
        }

        // 2. Try block-level continuity cache.
        let continuity_out = if let Ok(Some(block_entry)) =
            self.db.get_proofs_for_block(chain_key, header_number).await
        {
            if let Some(cp_json) = block_entry.continuity_proof.clone() {
                if let Ok(cp) = serde_json::from_value::<ContinuityProof>(cp_json) {
                    cp
                } else {
                    // Fall back to build if deserialization fails
                    self.build_continuity(chain_key, header_number).await?
                }
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
        if let Err(e) = self.db.try_insert_proofs_entry(entry).await {
            return Err(ServiceError::DbError {
                message: e.to_string(),
            });
        }

        // 6. Return response (cached=false because tx-specific entry was missing).
        Ok(ContinuityResponse {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: tx_hash_opt.map(|h| h256_to_hex(&h)),
            continuity_proof: continuity_out,
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(h256_to_hex(&merkle_root)),
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
    ) -> std::result::Result<ContinuityResponse, ServiceError> {
        // 0. Parse tx_hash
        let tx_h256 = self.parse_tx_hash(&tx_hash)?;

        // 1. Try DB lookup by tx_hash
        match self.db.get_proofs_by_tx_hash(chain_key, tx_h256).await {
            Ok(Some(entry)) => {
                // Deserialize cached proofs.
                let continuity_out_opt = entry
                    .continuity_proof
                    .as_ref()
                    .map(|v| serde_json::from_value::<ContinuityProof>(v.clone()))
                    .transpose()
                    .map_err(|e| {
                        tracing::error!(%tx_hash, chain_key, error=%e, "Cached continuity_proof deserialization failed");
                        ServiceError::DbError { message: e.to_string() }
                    })?;
                let merkle_out_opt = entry
                    .merkle_proof
                    .as_ref()
                    .map(|v| serde_json::from_value::<QueryMerkleProof>(v.clone()))
                    .transpose()
                    .map_err(|e| {
                        tracing::error!(%tx_hash, chain_key, error=%e, "Cached merkle_proof deserialization failed");
                        ServiceError::DbError { message: e.to_string() }
                    })?;

                if let (Some(continuity_out), Some(merkle_proof)) =
                    (continuity_out_opt.clone(), merkle_out_opt.clone())
                {
                    tracing::debug!(%tx_hash, chain_key, header_number=entry.header_number, "Cache hit: returning cached proofs");
                    return Ok(ContinuityResponse {
                        chain_key,
                        header_number: entry.header_number as u64,
                        tx_index: entry.tx_index.map(|i| i as u64),
                        tx_hash: Some(tx_hash),
                        continuity_proof: continuity_out,
                        merkle_proof: Some(merkle_proof.clone()),
                        merkle_root: entry.merkle_root.clone(),
                        cached: true,
                        generated_at: Utc::now(),
                    });
                }

                // Rebuild path: either missing continuity or merkle proof
                if let Some(tx_index_i64) = entry.tx_index {
                    tracing::debug!(%tx_hash, chain_key, header_number=entry.header_number, "Partial cache entry; rebuilding proofs");
                    let header_number_u64 = entry.header_number as u64;
                    let tx_index_u64 = tx_index_i64 as u64;
                    let rebuilt = self
                        .continuity_proof_with_tx(chain_key, header_number_u64, tx_index_u64)
                        .await?;
                    return Ok(ContinuityResponse {
                        cached: false,
                        ..rebuilt
                    });
                }

                // Neither full proofs nor tx index: treat as not found
                tracing::warn!(%tx_hash, chain_key, header_number=entry.header_number, "Cache entry missing required data for rebuild");
                Err(ServiceError::TxHashNotFound { tx_hash })
            }
            Ok(None) => Err(ServiceError::TxHashNotFound { tx_hash }),
            Err(e) => Err(ServiceError::DbError {
                message: e.to_string(),
            }),
        }
    }
}
