use std::time::Instant;

use super::*;
use attestor_primitives::block::ContinuityProof;

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
    /// <div class="warning">
    ///
    /// Note: Callers should validate the block via `validate_block_not_before_genesis`
    /// before calling this method. If the block is not yet attested, the builder
    /// will use "eager" proof generation with a predicted upper bound.
    ///
    /// </div>
    pub(crate) async fn build_continuity(
        &self,
        header_number: u64,
        current_block: u64,
    ) -> Result<ContinuityProof, ServiceError> {
        // Pass current_block to get_endpoints so it can validate predicted blocks immediately and fail fast
        let (lower_attestation, upper_attestation) = self
            .builder
            .get_endpoints(&[header_number], Some(current_block))
            .await
            .map_err(ServiceError::from)?;

        let proof = self
            .builder
            .build_for_single_query(header_number, lower_attestation.clone(), upper_attestation)
            .await
            .map_err(ServiceError::from)?;

        // Convert BuiltContinuityProof to ContinuityProof, preserving lower_endpoint_digest
        // Use to_attestor_proof_with_attestation_context to respect explicit lower_endpoint_digest
        // when proofs are trimmed (from_blocks_with_lower_digest), falling back to lower_attestation.digest
        // This is critical: trimmed proofs may have lower_endpoint_digest that differs from blocks[0].prev_digest
        let continuity_proof = proof
            .to_attestor_proof_with_attestation_context(lower_attestation.digest)
            .ok_or_else(|| ServiceError::Internal {
                message: format!(
                    "Failed to convert continuity proof to attestor proof format. Proof has {} blocks, lower_attestation digest: {:?}",
                    proof.blocks.len(),
                    lower_attestation.digest
                ),
            })?;

        Ok(continuity_proof)
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
        let merkle_start = Instant::now();

        // Fetch tx bytes & validate index.
        // Note: This uses Redis block caching if configured (via eth_client.get_block() -> block_cache.rs)
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
        // Note: This also uses Redis block caching if configured
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

        // Record merkle proof generation duration
        self.metrics
            .observe_merkle_generation(merkle_start.elapsed());

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
