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
    ///
    /// Note: Callers should validate the block via `validate_block_not_before_genesis`
    /// before calling this method. If the block is not yet attested, the builder
    /// will use "eager" proof generation with a predicted upper bound.
    pub(crate) async fn build_continuity(
        &self,
        header_number: u64,
    ) -> Result<(ContinuityProof, EndsInAttestation), ServiceError> {
        // TODO: Fetch these AttestationInfo from the postgres DB rather than getting them inefficiently
        let (lower_attestation, upper_attestation, ends_in_attestation) = self
            .builder
            .get_endpoints(&[header_number])
            .await
            .map_err(ServiceError::from)?;

        let proof = self
            .builder
            .build_for_single_query(header_number, lower_attestation, upper_attestation)
            .await
            .map_err(ServiceError::from)?;

        Ok((
            ContinuityProof::from_blocks(proof.blocks),
            ends_in_attestation,
        ))
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
        tx_index: Option<u64>,
    ) -> ServiceResult<ContinuityResponse> {
        // Generate continuity
        let (continuity_proof, ends_in_attestation) = self.build_continuity(header_number).await?;
        tracing::info!(
            chain_key,
            header_number,
            blocks = continuity_proof.roots.len(),
            "Continuity proof built successfully"
        );

        let continuity_item = ContinuityProofItem {
            chain_key,
            header_number,
            continuity_proof: continuity_proof.clone(),
            ends_in_attestation: ends_in_attestation.into(),
        };

        // Insert into DB asynchronously in background
        self.db.insert_continuity_proof(continuity_item.clone());

        match tx_index {
            Some(idx) => {
                let merkle = self
                    .generate_merkle_proof(chain_key, header_number, idx)
                    .await?;
                let mut response = build_response_from_proofs(merkle, continuity_item)?;
                response.cached = false;
                Ok(response)
            }
            None => Ok(ContinuityResponse {
                chain_key,
                header_number,
                tx_index: None,
                tx_hash: None,
                tx_bytes: None,
                continuity_proof,
                merkle_proof: None,
                cached: false,
                generated_at: Utc::now(),
            }),
        }
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
