use attestor_primitives::AttestationCheckpoint;

use super::*;

impl ContinuityService {
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
    pub(crate) async fn build_continuity(
        &self,
        header_number: u64,
    ) -> Result<(ContinuityProof, EndsInAttestation), ServiceError> {
        // TODO: Fetch these AttestationInfo from the postgres DB rather than getting them innefficiently
        let endpoints_result = self.builder.get_endpoints(&[header_number]).await;
        let (lower_attestation, upper_attestation, ends_in_attestation) = match endpoints_result {
            Ok(v) => v,
            Err(e) => return Err(self.handle_build_error_async(e, header_number).await),
        };

        let proof_result = self
            .builder
            .build_for_single_query(header_number, lower_attestation, upper_attestation)
            .await;
        let proof = match proof_result {
            Ok(v) => v,
            Err(e) => return Err(self.handle_build_error_async(e, header_number).await),
        };

        Ok((
            ContinuityProof::from_blocks(proof.blocks),
            ends_in_attestation,
        ))
    }

    /// Convert anyhow::Error from continuity builder to ServiceError with appropriate logging.
    /// Only fetches last_attested_block lazily when we're creating a BlockNotReady error.
    pub(crate) async fn handle_build_error_async(
        &self,
        error: anyhow::Error,
        query_block: u64,
    ) -> ServiceError {
        // Try to extract ContinuityError first (using downcast_ref to avoid moving)
        if let Some(continuity_err) = error.downcast_ref::<ContinuityError>() {
            // Special handling for BlockNotReady to add contextual logging
            if let ContinuityError::BlockNotReady {
                block_number,
                last_attested_block: err_last_attested,
            } = continuity_err
            {
                let is_query_block = *block_number == query_block;
                if is_query_block {
                    tracing::warn!(
                        query_block = *block_number,
                        last_attested_block = *err_last_attested,
                        "Query block is not attested to yet"
                    );
                } else {
                    tracing::warn!(
                        end_block = *block_number,
                        query_block,
                        last_attested_block = *err_last_attested,
                        "End block for continuity proof is not attested to yet"
                    );
                }
            }
            // Use From impl for all ContinuityError variants
            return ServiceError::from(continuity_err.clone());
        }

        // TODO: Update continuity builder to return proper ContinuityError variants instead of
        // anyhow::Error for these cases. Then we can match on error type instead of string content.
        // Currently checking error message strings, which is fragile.
        // Affected: continuity/src/builder/build.rs
        let error_msg = error.to_string();

        // Check for errors that indicate block is not attested yet
        let is_block_not_ready = error_msg.contains("Failed to get block")
            || error_msg.contains("No attestation or checkpoint found after block");

        if is_block_not_ready {
            // Only fetch last_attested_block when we actually need it for the error
            let last_attested_block = match self.builder.get_last_attested_block().await {
                Ok(maybe_block) => maybe_block.unwrap_or(0), // 0 indicates "no attestations yet"
                Err(e) => {
                    // If we can't get the last attested block, return RPC error instead
                    return ServiceError::RpcUnavailable {
                        message: format!(
                            "Block not ready and failed to get last attested block: {e}"
                        ),
                    };
                }
            };

            tracing::warn!(
                query_block,
                last_attested_block,
                "No attestation found after query block - block not attested yet"
            );
            return ServiceError::BlockNotReady {
                block_number: query_block,
                last_attested_block,
            };
        }

        // Fallback to generic internal error
        ServiceError::Internal { message: error_msg }
    }

    pub(crate) async fn fetch_continuity_by_height(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> ServiceResult<Option<ContinuityProofItem>> {
        let maybe_continuity = self
            .db
            .get_continuity_proof_for_block(chain_key, header_number)
            .await.map_err(|e| {
                tracing::error!(chain_key, header_number, error=%e, "Failed to fetch continuity proof by header_number and tx_index");
                ServiceError::DbError { message: e.to_string() }
            })?;
        let continuity_converted = maybe_continuity
            .map(ContinuityProofItem::try_from)
            .transpose().map_err(|e| {
                tracing::error!(chain_key, header_number, error=%e, "Failed to convert fetched continuity proof");
                ServiceError::DbError { message: e.to_string() }
            })?;

        Ok(continuity_converted)
    }

    pub(crate) async fn generate_and_cache_response(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<ContinuityResponse> {
        // Generate continuity
        let (continuity, ends_in_attestation) = self.build_continuity(header_number).await?;
        tracing::info!(
            chain_key,
            header_number,
            blocks = continuity.roots.len(),
            "Continuity proof built successfully"
        );

        let merkle = self
            .generate_merkle_proof(chain_key, header_number, tx_index)
            .await?;
        let continuity = ContinuityProofItem {
            chain_key,
            header_number,
            continuity_proof: continuity,
            ends_in_attestation: ends_in_attestation.into(),
        };

        // Insert into DB asynchronously in background
        self.db.insert_continuity_proof(continuity.clone());

        let mut generated = build_response_from_proofs(merkle, continuity)?;
        // Cached defaults to true, so we flip it
        generated.cached = false;

        Ok(generated)
    }

    pub(crate) async fn get_height_and_index_for_tx_hash(
        &self,
        tx_hash: H256,
    ) -> ServiceResult<(u64, u64)> {
        match self.builder.get_tx_position_by_hash(tx_hash).await {
            Ok((header_number, tx_index)) => Ok((header_number, tx_index)),
            Err(e) => Err(ServiceError::RpcUnavailable {
                message: format!("failed to resolve tx by hash via RPC: {e}"),
            }),
        }
    }

    pub(crate) async fn generate_merkle_proof(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<MerkleProofItem> {
        // Fetch tx bytes & validate index.
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

        // Merkle proof creation and tx hash computation.
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

        // Build Proof items for DB Insert
        let tx_bytes_for_cache = if tx_bytes.is_empty() {
            None
        } else {
            Some(tx_bytes[tx_index as usize].clone())
        };
        Ok(MerkleProofItem {
            chain_key,
            header_number,
            tx_index: Some(tx_index),
            tx_hash: tx_hash_opt,
            tx_bytes: tx_bytes_for_cache,
            merkle_proof,
            merkle_root,
        })
    }

    //
    pub(crate) async fn check_continuity_is_current(
        &self,
        continuity: &ContinuityProofItem,
    ) -> ServiceResult<bool> {
        // First check whether the continuity proof ends in a checkpoint. If so, it must still be verifyable
        if !continuity.ends_in_attestation {
            return Ok(true);
        };

        let last_checkpoint: Option<AttestationCheckpoint> = self
            .cc3_client
            .get_last_checkpoint(continuity.chain_key)
            .await
            .map_err(|e| ServiceError::RpcUnavailable {
                message: e.to_string(),
            })?;
        if let Some(last_checkpoint) = last_checkpoint {
            if continuity.continuity_proof.roots.is_empty() {
                return Ok(true);
            };
            // Continuity proof ordered so that roots[i] is at (queryHeight - 1) + i for single query
            // EX: query_height = 4, continuity proof with roots 3-10
            // continuity.header_number + continuity.continuity_proof.roots.len() - 2
            // = 4 + 8 - 2
            // = 10
            let continuity_max_height = (continuity.header_number
                + continuity.continuity_proof.roots.len() as u64)
                .checked_sub(2)
                .ok_or(ServiceError::Internal {
                    message: "Negative continuity_max_height. This shouldn't happen!".to_string(),
                })?;

            // If the highest block of the continuity proof is higher than the last checkpoint,
            // then the attestation it refers to must still be present on-chain. So the continuity
            // proof is verifyable.
            if last_checkpoint.block_number < continuity_max_height {
                Ok(true)
            } else {
                Ok(false)
            }
        } else {
            // No checkpoints. Continuity based on attestations will still be verifyable.
            Ok(true)
        }
    }

    pub(crate) async fn build_and_cache_continuity(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> ServiceResult<ContinuityResponse> {
        tracing::info!(
            chain_key,
            header_number,
            "Building continuity proof (cache miss)"
        );
        // TODO: Fetch these AttestationInfo from the postgres DB rather than getting them innefficiently
        let endpoints_result = self.builder.get_endpoints(&[header_number]).await;
        let (lower_attestation, upper_attestation, ends_in_attestation) = match endpoints_result {
            Ok(v) => v,
            Err(e) => return Err(self.handle_build_error_async(e, header_number).await),
        };

        let proof_result = self
            .builder
            .build_for_single_query(header_number, lower_attestation, upper_attestation)
            .await;
        let proof = match proof_result {
            Ok(v) => v,
            Err(e) => return Err(self.handle_build_error_async(e, header_number).await),
        };
        // Convert raw blocks into optimized ContinuityProof (attestor primitives)
        let continuity = ContinuityProof::from_blocks(proof.blocks);
        tracing::info!(
            chain_key,
            header_number,
            blocks = continuity.roots.len(),
            "Continuity proof built successfully"
        );

        // Insert into DB asynchronously in background
        let entry = ContinuityProofItem {
            chain_key,
            header_number,
            continuity_proof: continuity.clone(),
            ends_in_attestation: ends_in_attestation.into(),
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

pub(crate) fn parse_tx_hash(tx_hash: &str) -> Result<H256, ServiceError> {
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

pub(crate) fn build_response_from_proofs(
    merkle: MerkleProofItem,
    continuity: ContinuityProofItem,
) -> ServiceResult<ContinuityResponse> {
    let tx_hash = merkle.tx_hash;
    // We enforce here that if any tx specific field is in the DB entry, then all of them must be.
    // There is one exception. Block level proofs may use TX index 0 with other fields empty.
    let tx_index_some_non_zero = if let Some(index) = merkle.tx_index {
        index != 0
    } else {
        false
    };
    if (tx_hash.is_some() || tx_index_some_non_zero || merkle.tx_bytes.is_some())
        && !(tx_hash.is_some() && merkle.tx_index.is_some() && merkle.tx_bytes.is_some())
    {
        // If not all fields are present, we error out
        return Err(ServiceError::DbError { message: format!("Only some of tx-specific fields are present tx_hash: {tx_hash:?}, tx_index: {:?}, tx_bytes: {:?}", merkle.tx_index, merkle.tx_bytes) });
    }

    Ok(ContinuityResponse::from((merkle, continuity)))
}
