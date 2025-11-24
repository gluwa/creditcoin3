use crate::db::{DbManager, QueryProofs};
use attestor_primitives::block::ContinuityProof;
use attestor_primitives::Query;
use chrono::{DateTime, Utc};
use continuity::{ContinuityBuilder, ContinuityProof as RawContinuityProof};
use mmr::query_proof::QueryMerkleProof;
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::sync::Arc;

// === Serialization helpers ===
fn h256_to_hex(h: &H256) -> String {
    // still used for merkle proof output wrapping
    format!("0x{}", hex::encode(h.as_bytes()))
}

// Removed ContinuityBlockOut / ContinuityProofOut wrappers; we now reuse attestor primitives ContinuityProof directly.

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

    pub async fn continuity_proof(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> std::result::Result<ContinuityResponse, ServiceError> {
        // Attempt cache lookup
        let cached = match self.db.get_proofs_entry(chain_key, header_number).await {
            Ok(c) => c,
            Err(e) => {
                // Log DB error, continue as cache miss
                tracing::warn!(error=?e, "db read error for continuity cache");
                None
            }
        };
        if let Some(entry) = cached {
            if let Some(cp_json) = entry.continuity_proof {
                // Deserialize cached JSON continuity proof
                let continuity_proof_out: ContinuityProof = serde_json::from_value(cp_json)?;
                return Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: None,
                    tx_hash: None,
                    continuity_proof: continuity_proof_out,
                    merkle_proof: None,
                    merkle_root: entry.merkle_root, // already a hex string if present
                    cached: true,
                    generated_at: Utc::now(),
                });
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

        // Insert into DB (async)
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
    ) -> std::result::Result<ContinuityResponse, ServiceError> {
        // Build continuity proof (ignore cache for now for tx variant)
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
        let continuity_out = ContinuityProof::from_blocks(proof.blocks.clone());

        // Build a merkle proof for `tx_index` over the block's transaction bytes.
        // Mock providers yield deterministic fixture bytes; real providers yield live block transactions.
        // Returned siblings allow reconstruction of the merkle root.
        let tx_bytes = self
            .builder
            .get_block_tx_bytes(header_number)
            .await
            .map_err(|e| ServiceError::RpcUnavailable {
                message: e.to_string(),
            })?;

        if tx_bytes.is_empty() {
            // Allow tx_index == 0 for empty blocks and produce an empty merkle proof/root.
            if tx_index != 0 {
                return Err(ServiceError::TxIndexOutOfBounds { tx_index, len: 0 });
            }
        } else if tx_index as usize >= tx_bytes.len() {
            return Err(ServiceError::TxIndexOutOfBounds {
                tx_index,
                len: tx_bytes.len(),
            });
        }

        let tree = mmr::SimpleMerkleTree::new(&tx_bytes);
        let merkle_proof = if tx_bytes.is_empty() {
            QueryMerkleProof::new(tree.root(), vec![])
        } else {
            tree.generate_proof(tx_index as usize)
        };
        let merkle_root = tree.root();

        // Insert into DB
        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: None,
            continuity_proof: Some(continuity_out.clone()),
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(merkle_root),
        };
        self.db.insert_proofs_entry(entry);

        Ok(ContinuityResponse {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: None,
            continuity_proof: continuity_out,
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(h256_to_hex(&merkle_root)),
            cached: false,
            generated_at: Utc::now(),
        })
    }

    pub async fn continuity_proof_by_tx_hash(
        &self,
        chain_key: u64,
        tx_hash: String,
    ) -> std::result::Result<ContinuityResponse, ServiceError> {
        // Placeholder mapping: derive header_number & tx_index from hash bytes
        let header_number = 1; // TODO real lookup
        let tx_index = 0; // TODO real lookup
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
        let continuity_out = ContinuityProof::from_blocks(proof.blocks.clone());

        // If we had a mapping from tx_hash -> (header_number, tx_index) we'd call the tx_index variant.
        // For now, we try to fetch tx bytes and produce an empty-proof root if possible.
        let tx_bytes = self
            .builder
            .get_block_tx_bytes(header_number)
            .await
            .map_err(|e| ServiceError::RpcUnavailable {
                message: e.to_string(),
            })?;
        let tree = mmr::SimpleMerkleTree::new(&tx_bytes);
        let merkle_root = tree.root();
        let merkle_proof = QueryMerkleProof::new(merkle_root, vec![]);

        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: Some(H256::from_low_u64_be(0)), // placeholder
            continuity_proof: Some(continuity_out.clone()),
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(merkle_root),
        };
        self.db.insert_proofs_entry(entry);

        Ok(ContinuityResponse {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: Some(tx_hash),
            continuity_proof: continuity_out,
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(h256_to_hex(&merkle_root)),
            cached: false,
            generated_at: Utc::now(),
        })
    }
}
