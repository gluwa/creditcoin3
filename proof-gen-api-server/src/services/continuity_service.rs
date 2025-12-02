use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sp_core::H256;
use std::sync::Arc;

use crate::db::{DbManager, QueryProofs};
use attestor_primitives::block::ContinuityProof;
use continuity::{ContinuityBuilder, ContinuityError, ContinuityProof as RawContinuityProof};
use merkle::proof::TransactionMerkleProof;

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
        let builder_chain_key = self.builder.config.chain_key;
        if chain_key != builder_chain_key {
            return Err(ServiceError::InvalidParameter { message: format!("Chain key of requested proof doesn't match that supported by the continuity builder. Request key: {chain_key}, builder key: {builder_chain_key}") });
        }

        // Check if the requested block is available before attempting to build proof
        let current_block =
            self.builder
                .get_last_block()
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get current block height: {e}"),
                })?;

        if header_number > current_block {
            tracing::warn!(
                block_number = header_number,
                current_block,
                "Query block is not attested to yet"
            );
            return Err(ServiceError::BlockNotReady {
                block_number: header_number,
                current_block,
            });
        }

        let proof: RawContinuityProof = self
            .builder
            .build_for_single_query(header_number)
            .await
            .map_err(|e| self.handle_build_error(e, header_number, current_block))?;
        Ok(ContinuityProof::from_blocks(proof.blocks.clone()))
    }

    /// Convert anyhow::Error from continuity builder to ServiceError with appropriate logging
    fn handle_build_error(
        &self,
        error: anyhow::Error,
        query_block: u64,
        current_block: u64,
    ) -> ServiceError {
        // Try to extract ContinuityError first (using downcast_ref to avoid moving)
        if let Some(continuity_err) = error.downcast_ref::<ContinuityError>() {
            // Special handling for BlockNotReady to add contextual logging
            if let ContinuityError::BlockNotReady {
                block_number,
                current_block: err_current_block,
            } = continuity_err
            {
                let is_query_block = *block_number == query_block;
                if is_query_block {
                    tracing::warn!(
                        query_block = *block_number,
                        current_block = *err_current_block,
                        "Query block is not attested to yet"
                    );
                } else {
                    tracing::warn!(
                        end_block = *block_number,
                        query_block,
                        current_block = *err_current_block,
                        "End block for continuity proof is not attested to yet"
                    );
                }
            }
            // Use From impl for all ContinuityError variants
            return ServiceError::from(continuity_err.clone());
        }

        // Check for "Failed to get block" errors from eth client
        let error_msg = error.to_string();
        if error_msg.contains("Failed to get block") {
            tracing::warn!(
                query_block,
                current_block,
                "Query block is not attested to yet"
            );
            return ServiceError::BlockNotReady {
                block_number: query_block,
                current_block,
            };
        }

        // Fallback to generic internal error
        ServiceError::Internal { message: error_msg }
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
        let current_block =
            self.builder
                .get_last_block()
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get current block height: {e}"),
                })?;

        // Attempt cache lookup using block-level retrieval (tx_index IS NULL)
        match self.db.get_proofs_for_block(chain_key, header_number).await {
            Ok(Some(entry)) => {
                let proofs = QueryProofs::try_from(entry).map_err(|e| ServiceError::DbError {
                    message: e.to_string(),
                })?;
                if let Some(continuity_proof_out) = proofs.continuity_proof {
                    tracing::info!(
                        chain_key,
                        header_number,
                        "Cache hit: returning cached continuity proof"
                    );
                    return Ok(ContinuityResponse {
                        chain_key,
                        header_number,
                        tx_index: None,
                        tx_hash: None,
                        tx_bytes: None, // Block-level proof doesn't include tx bytes
                        continuity_proof: continuity_proof_out,
                        merkle_proof: None,
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
        let continuity_out = ContinuityProof::from_blocks(proof.blocks.clone());
        tracing::info!(
            chain_key,
            header_number,
            blocks = continuity_out.blocks.len(),
            "Continuity proof built successfully"
        );

        // Insert into DB asynchronously in background
        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: None,
            tx_hash: None,
            tx_bytes: None, // Block-level proof doesn't include tx bytes
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
            tx_bytes: None, // Block-level proof doesn't include tx bytes
            continuity_proof: continuity_out,
            merkle_proof: None,
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
                tracing::info!(
                    chain_key,
                    header_number,
                    tx_index,
                    "Cache hit: returning cached continuity proof with transaction"
                );
                // Use cached tx_bytes if available, otherwise fetch from RPC
                let tx_bytes_hex = if let Some(cached_bytes) = &proofs.tx_bytes {
                    Some(format!("0x{}", hex::encode(cached_bytes)))
                } else {
                    // Fallback: fetch from RPC if not cached
                    self.builder
                        .get_block_tx_bytes(header_number)
                        .await
                        .ok()
                        .and_then(|tx_bytes| {
                            let idx = tx_index as usize;
                            if idx < tx_bytes.len() {
                                Some(format!("0x{}", hex::encode(&tx_bytes[idx])))
                            } else {
                                None
                            }
                        })
                };
                return Ok(ContinuityResponse {
                    chain_key,
                    header_number,
                    tx_index: Some(tx_index),
                    tx_hash: proofs.tx_hash.map(|h| format!("0x{h:x}")),
                    tx_bytes: tx_bytes_hex,
                    continuity_proof: continuity_out,
                    merkle_proof: Some(merkle_proof.clone()),
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
                tracing::debug!(
                    chain_key,
                    header_number,
                    "Using cached continuity proof from block-level cache for tx-specific request"
                );
                cp
            } else {
                self.build_continuity(chain_key, header_number).await?
            }
        } else {
            tracing::info!(
                chain_key,
                header_number,
                tx_index,
                "Building continuity proof (cache miss)"
            );
            let built = self.build_continuity(chain_key, header_number).await?;
            tracing::info!(
                chain_key,
                header_number,
                blocks = built.blocks.len(),
                "Continuity proof built successfully"
            );
            built
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
        let tree = merkle::keccak_merkle_tree::KeccakMerkleTree::new(&tx_bytes);
        let merkle_proof = if tx_bytes.is_empty() {
            TransactionMerkleProof::new(tree.root(), vec![])
        } else {
            tree.generate_proof(tx_index as usize)
                .map_err(|e| ServiceError::MerkleError {
                    message: format!("{e:?}"),
                })?
        };
        let merkle_root = tree.root();

        // Get the actual transaction hash from the block (not computed from ABI-encoded bytes)
        // Ethereum transaction hashes are computed from RLP-encoded transactions, not ABI-encoded bytes
        let tx_hash_opt = if tx_bytes.is_empty() {
            None
        } else {
            self.builder
                .get_tx_hash_by_index(header_number, tx_index)
                .await
                .map_err(|e| ServiceError::RpcUnavailable {
                    message: format!("Failed to get tx hash: {e}"),
                })?
        };

        // 5. Insert tx-specific entry (fail fast if unavailable).
        let tx_bytes_for_cache = if tx_bytes.is_empty() {
            None
        } else {
            Some(tx_bytes[tx_index as usize].clone())
        };
        let entry = QueryProofs {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: tx_hash_opt,
            tx_bytes: tx_bytes_for_cache,
            continuity_proof: Some(continuity_out.clone()),
            merkle_proof: Some(merkle_proof.clone()),
            merkle_root: Some(merkle_root),
        };
        // Insert into DB asynchronously in background
        self.db.insert_proofs_entry(entry);

        // 6. Return response (cached=false because tx-specific entry was missing).
        // Include the transaction bytes used to generate the merkle proof
        let tx_bytes_hex = if tx_bytes.is_empty() {
            None
        } else {
            Some(format!("0x{}", hex::encode(&tx_bytes[tx_index as usize])))
        };
        Ok(ContinuityResponse {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: tx_hash_opt.map(|h| format!("0x{h:x}")),
            tx_bytes: tx_bytes_hex,
            continuity_proof: continuity_out,
            merkle_proof: Some(merkle_proof.clone()),
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
                    tracing::info!(%tx_hash, chain_key, header_number=header_number_u64, "Cache hit: returning cached proofs");
                    // Use cached tx_bytes if available, otherwise fetch from RPC
                    let tx_bytes_hex = if let Some(cached_bytes) = &proofs.tx_bytes {
                        Some(format!("0x{}", hex::encode(cached_bytes)))
                    } else if let Some(tx_index) = proofs.tx_index {
                        // Fallback: fetch from RPC if not cached
                        self.builder
                            .get_block_tx_bytes(header_number_u64)
                            .await
                            .ok()
                            .and_then(|tx_bytes| {
                                let idx = tx_index as usize;
                                if idx < tx_bytes.len() {
                                    Some(format!("0x{}", hex::encode(&tx_bytes[idx])))
                                } else {
                                    None
                                }
                            })
                    } else {
                        None
                    };
                    return Ok(ContinuityResponse {
                        chain_key,
                        header_number: header_number_u64,
                        tx_index: proofs.tx_index,
                        tx_hash: Some(tx_hash),
                        tx_bytes: tx_bytes_hex,
                        continuity_proof: continuity_out,
                        merkle_proof: Some(merkle_proof.clone()),
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

                    // Verify that the computed tx_hash matches the requested hash
                    if let Some(computed_hash) = &rebuilt.tx_hash {
                        let computed_h256 = self.parse_tx_hash(computed_hash)?;
                        if computed_h256 != tx_h256 {
                            return Err(ServiceError::TxHashNotFound {
                                tx_hash: format!(
                                    "Transaction hash mismatch: requested 0x{tx_h256:x}, but found {computed_hash} at block {header_number_u64} index {tx_index_u64}"
                                ),
                            });
                        }
                    }

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

                        // Verify that the computed tx_hash matches the requested hash
                        if let Some(computed_hash) = &generated.tx_hash {
                            let computed_h256 = self.parse_tx_hash(computed_hash)?;
                            if computed_h256 != tx_h256 {
                                return Err(ServiceError::TxHashNotFound {
                                    tx_hash: format!(
                                        "Transaction hash mismatch: requested 0x{tx_h256:x}, but found {computed_hash} at block {header_number} index {tx_index}"
                                    ),
                                });
                            }
                        }

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
