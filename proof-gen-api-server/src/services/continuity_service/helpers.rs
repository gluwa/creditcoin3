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
    /// Build a continuity proof.
    ///
    /// Flow:
    /// 1. Fetch checkpoint boundaries from indexer (GraphQL)
    /// 2. Try building proof from GraphQL attestation data
    /// 3. If GraphQL proof fails → fall back to eth provider (archiver/chain) for roots
    /// 4. If indexer unavailable → fall back to CC3 for boundaries + eth provider for roots
    pub(crate) async fn build_continuity(
        &self,
        header_numbers: &[u64],
        current_block: u64,
    ) -> Result<ContinuityProof, ServiceError> {
        let (&min_query, &max_query) = header_numbers
            .iter()
            .min()
            .zip(header_numbers.iter().max())
            .ok_or(ServiceError::Internal {
                message: "header_numbers is empty".into(),
            })?;

        // Step 1: Fetch boundaries from indexer (GraphQL).
        let boundaries = self
            .resolve_checkpoint_boundaries_from_indexer(min_query, max_query)
            .await;

        let (lower, lower_digest, upper) = match boundaries {
            Ok(b) => b,
            Err(indexer_err) => {
                // Step 4: Indexer unavailable → fall back to CC3 for boundaries.
                tracing::warn!(
                    error = %indexer_err,
                    "indexer unavailable, falling back to CC3 for boundaries"
                );
                self.resolve_checkpoint_boundaries_from_cc3(min_query, max_query)
                    .await?
            }
        };

        // Step 2: Try building proof from GraphQL attestation data.
        match self
            .build_proof_from_graphql(header_numbers, current_block)
            .await
        {
            Ok(proof) => Ok(proof),
            Err(e) => {
                // Step 3: GraphQL proof failed → fall back to eth provider for roots.
                tracing::warn!(
                    error = %e,
                    "GraphQL proof failed, falling back to eth provider"
                );
                self.build_proof_from_roots(min_query, lower, lower_digest, upper)
                    .await
            }
        }
    }

    /// Try building a proof from GraphQL attestation data.
    async fn build_proof_from_graphql(
        &self,
        header_numbers: &[u64],
        current_block: u64,
    ) -> Result<ContinuityProof, ServiceError> {
        let (lower_attestation, upper_attestation) = self
            .builder
            .get_endpoints(header_numbers, Some(current_block))
            .await
            .map_err(ServiceError::from)?;

        let proof = self
            .builder
            .build_for_batch_queries(header_numbers, lower_attestation.clone(), upper_attestation)
            .await
            .map_err(ServiceError::from)?;

        proof
            .to_attestor_proof_with_attestation_context(lower_attestation.digest)
            .ok_or_else(|| ServiceError::Internal {
                message: format!(
                    "Failed to convert proof: {} blocks, lower digest: {:?}",
                    proof.blocks.len(),
                    lower_attestation.digest
                ),
            })
    }

    /// Build a proof from eth provider roots (archiver or chain) using
    /// pre-resolved checkpoint boundaries. The `lower_checkpoint_digest` is
    /// the on-chain digest at the lower checkpoint block — this anchors the
    /// proof's digest chain to a known on-chain value.
    async fn build_proof_from_roots(
        &self,
        min_query: u64,
        lower_checkpoint: u64,
        lower_checkpoint_digest: sp_core::H256,
        upper_checkpoint: u64,
    ) -> Result<ContinuityProof, ServiceError> {
        tracing::info!(
            lower_checkpoint,
            ?lower_checkpoint_digest,
            upper_checkpoint,
            min_query,
            "building proof from eth provider roots"
        );

        // Start the digest chain from the lower checkpoint's known on-chain digest.
        // Blocks are built from lower_checkpoint + 1 since the checkpoint block itself
        // is already accounted for by its digest.
        let build_from = lower_checkpoint + 1;
        let blocks = self
            .builder
            .eth_provider
            .build_continuity_blocks(lower_checkpoint_digest, build_from, upper_checkpoint)
            .await
            .map_err(|err| ServiceError::Internal {
                message: format!("failed to build continuity blocks: {err}"),
            })?;

        let lower_endpoint_digest = blocks
            .iter()
            .take_while(|b| b.n() < min_query)
            .last()
            .map(|b| b.digest())
            .unwrap_or(lower_checkpoint_digest);

        let proof_roots: Vec<sp_core::H256> = blocks
            .iter()
            .filter(|b| b.n() >= min_query)
            .map(|b| b.root)
            .collect();

        Ok(ContinuityProof::new(lower_endpoint_digest, proof_roots))
    }

    /// Fetch checkpoint boundaries from the indexer (GraphQL).
    /// Returns `(lower_block, lower_digest, upper_block)`.
    async fn resolve_checkpoint_boundaries_from_indexer(
        &self,
        min_query: u64,
        max_query: u64,
    ) -> Result<(u64, sp_core::H256, u64), ServiceError> {
        let indexer =
            self.builder
                .indexer_provider
                .as_ref()
                .ok_or_else(|| ServiceError::Internal {
                    message: "no indexer configured".into(),
                })?;

        let chain_key = self.builder.config.chain_key;
        let max_range = self.builder.config.checkpoint_block_interval() * 2;

        let checkpoints = indexer
            .get_checkpoints_around_height(chain_key, min_query, max_range)
            .await
            .map_err(|err| ServiceError::Internal {
                message: format!("failed to fetch checkpoints from indexer: {err}"),
            })?;

        Self::extract_boundaries(&checkpoints, min_query, max_query)
    }

    /// Fetch checkpoint boundaries from CC3 (on-chain).
    /// Returns `(lower_block, lower_digest, upper_block)`.
    async fn resolve_checkpoint_boundaries_from_cc3(
        &self,
        min_query: u64,
        max_query: u64,
    ) -> Result<(u64, sp_core::H256, u64), ServiceError> {
        let chain_key = self.builder.config.chain_key;

        let checkpoints = self
            .builder
            .cc_provider
            .get_checkpoints_for_chain(chain_key)
            .await
            .map_err(|err| ServiceError::Internal {
                message: format!("failed to fetch checkpoints from CC3: {err}"),
            })?;

        Self::extract_boundaries(&checkpoints, min_query, max_query)
    }

    /// Find the nearest lower and upper checkpoint boundaries around a query range.
    /// Returns `(lower_block, lower_digest, upper_block)`.
    fn extract_boundaries(
        checkpoints: &[attestor_primitives::AttestationCheckpoint],
        min_query: u64,
        max_query: u64,
    ) -> Result<(u64, sp_core::H256, u64), ServiceError> {
        let lower = checkpoints
            .iter()
            .filter(|cp| cp.block_number <= min_query)
            .max_by_key(|cp| cp.block_number);
        let upper = checkpoints
            .iter()
            .filter(|cp| cp.block_number >= max_query)
            .min_by_key(|cp| cp.block_number);

        match (lower, upper) {
            (Some(l), Some(u)) => Ok((l.block_number, l.digest, u.block_number)),
            _ => Err(ServiceError::Internal {
                message: format!(
                    "no checkpoints found around query range {min_query}..{max_query}"
                ),
            }),
        }
    }

    pub(crate) async fn get_height_and_index_for_tx_hash(
        &self,
        tx_hash: H256,
    ) -> ServiceResult<(u64, u64)> {
        match self.builder.get_tx_position_by_hash(tx_hash).await {
            Ok(Some((header_number, tx_index))) => Ok((header_number, tx_index)),
            Ok(None) => Err(ServiceError::TxHashNotFound {
                tx_hash: format!("0x{}", hex::encode(tx_hash.as_bytes())),
            }),
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
                return Err(ServiceError::TxIndexOutOfBounds {
                    height: header_number,
                    tx_index,
                    len: 0,
                });
            }
        } else if tx_index as usize >= tx_bytes.len() {
            return Err(ServiceError::TxIndexOutOfBounds {
                height: header_number,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prom::NoopMetrics;
    use anyhow::Result;
    use async_trait::async_trait;
    use attestor_primitives::block::Block;
    use continuity::rpc::EthRpcProvider;
    use continuity::{mocks::make_mock_providers, ContinuityBuilder, ContinuityConfig};
    use std::sync::Arc;

    struct FoundEthProvider;

    #[async_trait]
    impl EthRpcProvider for FoundEthProvider {
        async fn build_continuity_blocks(
            &self,
            _lower_digest: H256,
            _start: u64,
            _end: u64,
        ) -> Result<Vec<Block>> {
            Ok(vec![])
        }
        async fn get_block_tx_bytes(&self, _block_number: u64) -> Result<Vec<Vec<u8>>> {
            Ok(vec![])
        }
        async fn get_tx_hash_by_index(
            &self,
            _block_number: u64,
            _tx_index: u64,
        ) -> Result<Option<H256>> {
            Ok(None)
        }
        async fn get_tx_position_by_hash(&self, _tx_hash: H256) -> Result<Option<(u64, u64)>> {
            Ok(Some((42, 3)))
        }
        async fn get_last_block(&self) -> Result<u64> {
            Ok(1000)
        }
        async fn get_chain_id(&self) -> Result<u64> {
            Ok(31337)
        }
    }

    struct ErrorEthProvider;

    #[async_trait]
    impl EthRpcProvider for ErrorEthProvider {
        async fn build_continuity_blocks(
            &self,
            _lower_digest: H256,
            _start: u64,
            _end: u64,
        ) -> Result<Vec<Block>> {
            Ok(vec![])
        }
        async fn get_block_tx_bytes(&self, _block_number: u64) -> Result<Vec<Vec<u8>>> {
            Ok(vec![])
        }
        async fn get_tx_hash_by_index(
            &self,
            _block_number: u64,
            _tx_index: u64,
        ) -> Result<Option<H256>> {
            Ok(None)
        }
        async fn get_tx_position_by_hash(&self, _tx_hash: H256) -> Result<Option<(u64, u64)>> {
            Err(anyhow::anyhow!("connection refused"))
        }
        async fn get_last_block(&self) -> Result<u64> {
            Ok(1000)
        }
        async fn get_chain_id(&self) -> Result<u64> {
            Ok(31337)
        }
    }

    fn mock_config(chain_key: u64) -> ContinuityConfig {
        ContinuityConfig::builder()
            .cc3_rpc_url("ws://mock")
            .eth_rpc_url("http://mock")
            .chain_key(chain_key)
            .attestation_interval(10)
            .checkpoint_interval(10)
            .build()
    }

    async fn make_service(eth_provider: Arc<dyn EthRpcProvider>) -> ContinuityService {
        let chain_key = 2;
        let (cc_provider, _) = make_mock_providers(chain_key);
        let builder = Arc::new(ContinuityBuilder::new_with_providers(
            mock_config(chain_key),
            cc_provider,
            eth_provider,
        ));
        ContinuityService::new(builder, NoopMetrics::new(), 10)
            .await
            .expect("service init should succeed with mocks")
    }

    #[tokio::test]
    async fn tx_hash_found_returns_position() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let hash = H256::from_low_u64_be(1);
        let result = svc.get_height_and_index_for_tx_hash(hash).await;
        assert_eq!(result.unwrap(), (42, 3));
    }

    #[tokio::test]
    async fn tx_hash_not_found_returns_not_found_error() {
        let (_, eth_provider) = make_mock_providers(2);
        let svc = make_service(eth_provider).await;
        let hash = H256::from_low_u64_be(999);
        let result = svc.get_height_and_index_for_tx_hash(hash).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, ServiceError::TxHashNotFound { .. }),
            "expected TxHashNotFound, got {err:?}"
        );
        assert!(!err.retriable());
        assert_eq!(err.code(), "TxHashNotFound");
    }

    #[tokio::test]
    async fn tx_hash_rpc_error_returns_rpc_unavailable() {
        let svc = make_service(Arc::new(ErrorEthProvider)).await;
        let hash = H256::from_low_u64_be(1);
        let result = svc.get_height_and_index_for_tx_hash(hash).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, ServiceError::RpcUnavailable { .. }),
            "expected RpcUnavailable, got {err:?}"
        );
        assert!(err.retriable());
        assert_eq!(err.code(), "RpcUnavailable");
    }
}
