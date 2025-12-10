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
    ) -> Result<ContinuityProof, ServiceError> {
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
    pub(crate) fn handle_build_error(
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

    pub(crate) async fn fetch_continuity_by_height(
        &self,
        chain_key: u64,
        header_number: u64,
    ) -> Result<Option<ContinuityProofItem>> {
        let maybe_continuity = self
            .db
            .get_continuity_proof_for_block(chain_key, header_number)
            .await?;
        let continuity_converted = maybe_continuity
            .map(ContinuityProofItem::try_from)
            .transpose()?;

        Ok(continuity_converted)
    }

    pub(crate) async fn generate_and_cache_response(
        &self,
        chain_key: u64,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<ContinuityResponse> {
        // Generate continuity
        let continuity = self.build_continuity(header_number).await?;
        tracing::info!(
            chain_key,
            header_number,
            blocks = continuity.blocks.len(),
            "Continuity proof built successfully"
        );

        let merkle = self
            .generate_merkle_proof(chain_key, header_number, tx_index)
            .await?;
        let continuity = ContinuityProofItem {
            chain_key,
            header_number,
            continuity_proof: continuity,
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
    let (tx_hash, chain_key, header_number) =
        (merkle.tx_hash, merkle.chain_key, merkle.header_number);
    tracing::info!(
        chain_key,
        header_number,
        "Cache hit: returning cached proofs. Tx_hash: {tx_hash:?}"
    );
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
