use std::sync::Arc;
use std::time::Instant;

use super::*;
use anyhow::Error as AnyhowError;
use attestor_primitives::block::ContinuityProof;
use eth;

/// If the anyhow chain wraps an [`eth::Error`] that the eth client classifies as a
/// payload-inconsistency case ([`eth::Error::inconsistent_block_payload_for_fallback`]),
/// return the corresponding `ServiceError::UnsupportedBlockFormat`, using the block number
/// hint from the chain when present and `fallback_block_number` otherwise.
///
/// Otherwise, return `None` and let the caller pick the appropriate non-inconsistent
/// mapping (typically `RpcUnavailable` or `Internal`).
fn classify_eth_rpc_anyhow_as_inconsistent(
    err: &AnyhowError,
    fallback_block_number: u64,
) -> Option<ServiceError> {
    if !eth::anyhow_chain_is_inconsistent_block_payload(err) {
        return None;
    }
    let block_number =
        eth::anyhow_chain_inconsistent_block_number_hint(err).unwrap_or(fallback_block_number);
    Some(ServiceError::UnsupportedBlockFormat { block_number })
}

/// Map an `anyhow::Error` returned by the eth provider into a `ServiceError`,
/// using `UnsupportedBlockFormat` for payload-inconsistency causes and
/// `RpcUnavailable` for everything else. Use
/// [`map_eth_rpc_anyhow_to_service_error_with`] when the non-inconsistent fallback
/// should be a different variant (e.g. `Internal`).
fn map_eth_rpc_anyhow_to_service_error(
    err: AnyhowError,
    fallback_block_number: u64,
) -> ServiceError {
    map_eth_rpc_anyhow_to_service_error_with(err, fallback_block_number, |err| {
        ServiceError::RpcUnavailable {
            message: err.to_string(),
        }
    })
}

/// Same as [`map_eth_rpc_anyhow_to_service_error`] but lets the caller supply the
/// non-inconsistent fallback mapping. Keeps the payload-inconsistency detection in
/// a single place so a future addition to
/// [`eth::Error::inconsistent_block_payload_for_fallback`] only needs to be wired
/// up once.
fn map_eth_rpc_anyhow_to_service_error_with<F>(
    err: AnyhowError,
    fallback_block_number: u64,
    on_other: F,
) -> ServiceError
where
    F: FnOnce(AnyhowError) -> ServiceError,
{
    if let Some(svc_err) = classify_eth_rpc_anyhow_as_inconsistent(&err, fallback_block_number) {
        return svc_err;
    }
    on_other(err)
}

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
    /// 1. Resolve checkpoint boundaries from local cache. Try attestation
    ///    bounds first (tightest), then checkpoint bounds, then a mixed
    ///    cross-cache bracket as a final fallback for the steady-state gap
    ///    that follows a checkpoint prune (see [`ContinuityService::get_mixed_boundaries`]).
    /// 2. Build proof from eth provider roots (archiver or chain)
    pub(crate) async fn build_continuity(
        &self,
        chain: &Arc<ChainState>,
        header_numbers: &[u64],
    ) -> Result<ContinuityProof, ServiceError> {
        let (&min_query, &max_query) = header_numbers
            .iter()
            .min()
            .zip(header_numbers.iter().max())
            .ok_or(ServiceError::Internal {
                message: "header_numbers is empty".into(),
            })?;

        // Step 1: Resolve boundaries from local caches.
        // Try attestation bounds first (more granular), fall back to checkpoint
        // bounds, and finally to a mixed (one half from each cache) bracket
        // before bailing.
        let (lower, lower_digest, upper, upper_digest) = if let Some(bounds) = self
            .get_attestation_boundaries(chain, min_query, max_query)
            .await
        {
            tracing::debug!(
                min_query,
                max_query,
                lower = bounds.0,
                upper = bounds.2,
                "🔧 🔍 resolved boundaries from attestation cache"
            );
            bounds
        } else if let Some(bounds) = self
            .get_checkpoint_boundaries(chain, min_query, max_query)
            .await
        {
            tracing::debug!(
                min_query,
                max_query,
                lower = bounds.0,
                upper = bounds.2,
                "🔧 🔍 resolved boundaries from checkpoint cache"
            );
            bounds
        } else if let Some(bounds) = self.get_mixed_boundaries(chain, min_query, max_query).await {
            tracing::debug!(
                min_query,
                max_query,
                lower = bounds.0,
                upper = bounds.2,
                "🔧 🔍 resolved boundaries from mixed cache bracket (one half per cache)"
            );
            bounds
        } else {
            return Err(self
                .boundary_lookup_failed_error(chain, min_query, max_query)
                .await);
        };

        // Step 2: Build directly from eth provider (archiver or chain).
        self.build_proof_from_roots(chain, min_query, lower, lower_digest, upper, upper_digest)
            .await
    }

    /// Build a proof from eth provider roots (archiver or chain) using
    /// pre-resolved checkpoint boundaries. The `lower_checkpoint_digest` is
    /// the on-chain digest at the lower checkpoint block — this anchors the
    /// proof's digest chain to a known on-chain value. The `upper_checkpoint_digest`
    /// is the on-chain digest at the upper checkpoint block; after building, the
    /// digest computed by walking the proof chain up to `upper_checkpoint` is
    /// verified against this value. A mismatch means the eth provider returned
    /// roots that do not reconcile with the on-chain upper boundary (stale data,
    /// reorg, provider bug, etc.) and the proof must be rejected rather than served.
    async fn build_proof_from_roots(
        &self,
        chain_state: &Arc<ChainState>,
        min_query: u64,
        lower_checkpoint: u64,
        lower_checkpoint_digest: sp_core::H256,
        upper_checkpoint: u64,
        upper_checkpoint_digest: sp_core::H256,
    ) -> Result<ContinuityProof, ServiceError> {
        tracing::info!(
            lower_checkpoint,
            ?lower_checkpoint_digest,
            upper_checkpoint,
            ?upper_checkpoint_digest,
            min_query,
            "🔧 🛠️  building proof from eth provider roots"
        );

        // Start the digest chain from the lower checkpoint's known on-chain digest.
        // Blocks are built from lower_checkpoint + 1 since the checkpoint block itself
        // is already accounted for by its digest.
        let build_from = lower_checkpoint + 1;
        let blocks = chain_state
            .builder
            .eth_provider
            .build_continuity_blocks(lower_checkpoint_digest, build_from, upper_checkpoint)
            .await
            .map_err(|err| {
                map_eth_rpc_anyhow_to_service_error_with(err, min_query, |err| {
                    ServiceError::Internal {
                        message: format!("failed to build continuity blocks: {err}"),
                    }
                })
            })?;

        // Verify the built chain reconciles with the on-chain upper boundary digest.
        // Without this check, the proof's upper end is only tied to the upper *block number*,
        // and a faulty/stale eth provider could return roots that build a digest chain
        // unrelated to the real attested/checkpointed state.
        let computed_upper_digest = blocks
            .iter()
            .find(|b| b.n() == upper_checkpoint)
            .map(|b| b.digest());
        match computed_upper_digest {
            Some(d) if d == upper_checkpoint_digest => {}
            Some(d) => {
                tracing::error!(
                    upper_checkpoint,
                    expected = ?upper_checkpoint_digest,
                    computed = ?d,
                    "🔧 ❌ continuity upper-boundary digest mismatch"
                );
                return Err(ServiceError::Internal {
                    message: format!(
                        "continuity upper-boundary digest mismatch at block {upper_checkpoint}: \
                         expected {upper_checkpoint_digest:?}, computed {d:?}"
                    ),
                });
            }
            None => {
                tracing::error!(
                    upper_checkpoint,
                    build_from,
                    n_blocks = blocks.len(),
                    "🔧 ❌ eth provider returned no block at upper checkpoint boundary"
                );
                return Err(ServiceError::Internal {
                    message: format!(
                        "eth provider returned no block at upper checkpoint boundary {upper_checkpoint} \
                         (range {build_from}..={upper_checkpoint}, got {} blocks)",
                        blocks.len()
                    ),
                });
            }
        }

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

    pub(crate) async fn get_height_and_index_for_tx_hash(
        &self,
        chain: &Arc<ChainState>,
        tx_hash: H256,
    ) -> ServiceResult<(u64, u64)> {
        match chain.builder.get_tx_position_by_hash(tx_hash).await {
            Ok(Some((header_number, tx_index))) => Ok((header_number, tx_index)),
            Ok(None) => Err(ServiceError::TxHashNotFound {
                tx_hash: format!("0x{}", hex::encode(tx_hash.as_bytes())),
            }),
            Err(e) => Err(map_eth_rpc_anyhow_to_service_error(e, 0)),
        }
    }

    pub(crate) async fn generate_merkle_proof(
        &self,
        chain: &Arc<ChainState>,
        header_number: u64,
        tx_index: u64,
    ) -> ServiceResult<MerkleProofItem> {
        let chain_key = chain.builder.config.chain_key;
        let merkle_start = Instant::now();

        // Fetch tx bytes & validate index.
        // Note: This uses Redis block caching if configured (via eth_client.get_block() -> block_cache.rs)
        let tx_bytes = chain
            .builder
            .get_block_tx_bytes(header_number)
            .await
            .map_err(|e| map_eth_rpc_anyhow_to_service_error(e, header_number))?;
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
            chain
                .builder
                .get_tx_hash_by_index(header_number, tx_index)
                .await
                .map_err(|e| map_eth_rpc_anyhow_to_service_error(e, header_number))?
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

        async fn is_healthy(&self) -> Result<bool> {
            Ok(true)
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

        async fn is_healthy(&self) -> Result<bool> {
            Err(anyhow::anyhow!("connection refused"))
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
        make_service_with_batch_span(eth_provider, 1_000).await
    }

    async fn make_service_with_batch_span(
        eth_provider: Arc<dyn EthRpcProvider>,
        max_batch_span: u64,
    ) -> ContinuityService {
        let chain_key = 2;
        let (cc_provider, _) = make_mock_providers(chain_key);
        let builder = Arc::new(ContinuityBuilder::new_with_providers(
            mock_config(chain_key),
            cc_provider,
            eth_provider,
        ));
        ContinuityService::new(vec![builder], NoopMetrics::new(), 10, max_batch_span)
            .await
            .expect("service init should succeed with mocks")
    }

    #[tokio::test]
    async fn tx_hash_found_returns_position() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let hash = H256::from_low_u64_be(1);
        let chain = svc.chain_state(2).unwrap();
        let result = svc.get_height_and_index_for_tx_hash(chain, hash).await;
        assert_eq!(result.unwrap(), (42, 3));
    }

    #[tokio::test]
    async fn tx_hash_not_found_returns_not_found_error() {
        let (_, eth_provider) = make_mock_providers(2);
        let svc = make_service(eth_provider).await;
        let hash = H256::from_low_u64_be(999);
        let chain = svc.chain_state(2).unwrap();
        let result = svc.get_height_and_index_for_tx_hash(chain, hash).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, ServiceError::TxHashNotFound { .. }),
            "expected TxHashNotFound, got {err:?}"
        );
        assert!(!err.retriable());
        assert_eq!(err.code(), "TxHashNotFound");
    }

    // ---------------------------------------------------------------------
    // Tests for the eth::Error -> ServiceError mapping helper.
    //
    // The classifier is the single source of truth for which `eth::Error`
    // variants map to `UnsupportedBlockFormat` (422, non-retriable) versus
    // the caller-supplied fallback (typically `RpcUnavailable` 503 or
    // `Internal` 500). Keep these tests in lockstep with
    // `eth::Error::inconsistent_block_payload_for_fallback`.
    // ---------------------------------------------------------------------

    #[test]
    fn map_eth_rpc_anyhow_maps_inconsistent_variants_to_unsupported() {
        for variant in [
            eth::Error::BlockHeaderRootsMismatch(11),
            eth::Error::TransactionsReceiptsMismatch(12),
            eth::Error::NotFullTransactionsFetched(13),
        ] {
            let block = variant
                .inconsistent_block_number_hint()
                .expect("variant must expose a block-number hint");
            let err = anyhow::Error::new(variant);
            let svc_err = super::map_eth_rpc_anyhow_to_service_error(err, /* fallback */ 999);
            match svc_err {
                ServiceError::UnsupportedBlockFormat { block_number } => {
                    assert_eq!(
                        block_number, block,
                        "block number hint must win over the fallback"
                    );
                }
                other => panic!("expected UnsupportedBlockFormat, got {other:?}"),
            }
        }
    }

    #[test]
    fn map_eth_rpc_anyhow_uses_fallback_block_when_hint_missing() {
        // A wrapped error with no `eth::Error` cause should fall through to
        // RpcUnavailable, *not* UnsupportedBlockFormat with a guessed block.
        let err = anyhow::anyhow!("transport failure, no eth::Error in chain");
        let svc_err = super::map_eth_rpc_anyhow_to_service_error(err, 12345);
        assert!(
            matches!(svc_err, ServiceError::RpcUnavailable { .. }),
            "non-eth-Error chains must fall through to RpcUnavailable, got {svc_err:?}"
        );
    }

    #[test]
    fn map_eth_rpc_anyhow_treats_failed_to_get_block_as_rpc_unavailable() {
        // Regression: `FailedToGetBlock` / `FailedToGetReceipts` are produced when
        // a provider answers `Ok(None)` (block not yet present). They must NOT map
        // to UnsupportedBlockFormat (which would be a non-retriable 422 to the
        // client). They should fall through to the caller-supplied fallback
        // mapping — here, `RpcUnavailable`.
        let err = anyhow::Error::new(eth::Error::FailedToGetBlock(42));
        let svc_err = super::map_eth_rpc_anyhow_to_service_error(err, 42);
        assert!(
            matches!(svc_err, ServiceError::RpcUnavailable { .. }),
            "FailedToGetBlock must map to RpcUnavailable, got {svc_err:?}"
        );
        assert!(svc_err.retriable());

        let err = anyhow::Error::new(eth::Error::FailedToGetReceipts(42));
        let svc_err = super::map_eth_rpc_anyhow_to_service_error(err, 42);
        assert!(
            matches!(svc_err, ServiceError::RpcUnavailable { .. }),
            "FailedToGetReceipts must map to RpcUnavailable, got {svc_err:?}"
        );
        assert!(svc_err.retriable());
    }

    #[test]
    fn map_eth_rpc_anyhow_with_uses_caller_fallback_for_non_inconsistent() {
        let err = anyhow::anyhow!("some non-eth failure");
        let svc_err =
            super::map_eth_rpc_anyhow_to_service_error_with(err, 7, |e| ServiceError::Internal {
                message: format!("build failed: {e}"),
            });
        match svc_err {
            ServiceError::Internal { message } => assert!(
                message.starts_with("build failed:"),
                "caller fallback must be invoked, got {message:?}"
            ),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[test]
    fn map_eth_rpc_anyhow_with_still_routes_inconsistent_to_unsupported() {
        // The caller-supplied fallback must not override the inconsistent path.
        // Even when the caller wants `Internal` for the non-inconsistent case,
        // a genuine mismatch is still surfaced as `UnsupportedBlockFormat`.
        let err = anyhow::Error::new(eth::Error::BlockHeaderRootsMismatch(50));
        let svc_err =
            super::map_eth_rpc_anyhow_to_service_error_with(err, 999, |e| ServiceError::Internal {
                message: e.to_string(),
            });
        match svc_err {
            ServiceError::UnsupportedBlockFormat { block_number } => {
                assert_eq!(block_number, 50);
            }
            other => panic!("expected UnsupportedBlockFormat, got {other:?}"),
        }
    }

    #[test]
    fn map_eth_rpc_anyhow_walks_context_chain_to_find_eth_error() {
        // The eth::Error often sits behind several `.context(...)` calls
        // before reaching the service layer. The classifier walks the full
        // error chain via `anyhow::Error::chain()`, so a wrapped variant must
        // still be recognized.
        let inner = anyhow::Error::new(eth::Error::TransactionsReceiptsMismatch(77));
        let wrapped = inner.context("while building continuity blocks");
        let svc_err = super::map_eth_rpc_anyhow_to_service_error(wrapped, /* fallback */ 1);
        match svc_err {
            ServiceError::UnsupportedBlockFormat { block_number } => {
                assert_eq!(
                    block_number, 77,
                    "block number must be lifted from the deep cause"
                );
            }
            other => panic!("expected UnsupportedBlockFormat, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tx_hash_rpc_error_returns_rpc_unavailable() {
        let svc = make_service(Arc::new(ErrorEthProvider)).await;
        let hash = H256::from_low_u64_be(1);
        let chain = svc.chain_state(2).unwrap();
        let result = svc.get_height_and_index_for_tx_hash(chain, hash).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err, ServiceError::RpcUnavailable { .. }),
            "expected RpcUnavailable, got {err:?}"
        );
        assert!(err.retriable());
        assert_eq!(err.code(), "RpcUnavailable");
    }

    #[tokio::test]
    async fn attestation_boundaries_preferred_over_checkpoint() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;

        // Mock has attestations at 10, 20, 30, ..., 1000
        // and checkpoints at 0, 100, 200, ..., 1000
        // For query block 15, attestation bounds should be (10, 20)
        // which is tighter than checkpoint bounds (0, 100)
        let chain = svc.chain_state(2).unwrap();
        let att_bounds = svc.get_attestation_boundaries(chain, 15, 15).await;
        assert!(att_bounds.is_some(), "attestation bounds should exist");
        let (lower, _, upper, upper_digest) = att_bounds.unwrap();
        assert_eq!(lower, 10);
        assert_eq!(upper, 20);
        assert_ne!(
            upper_digest,
            H256::zero(),
            "upper attestation digest should be non-zero"
        );

        let cp_bounds = svc.get_checkpoint_boundaries(chain, 15, 15).await;
        assert!(cp_bounds.is_some(), "checkpoint bounds should exist");
        let (lower, _, upper, upper_digest) = cp_bounds.unwrap();
        assert_eq!(lower, 0);
        assert_eq!(upper, 100);
        assert_ne!(
            upper_digest,
            H256::zero(),
            "upper checkpoint digest should be non-zero"
        );
    }

    #[tokio::test]
    async fn attestation_boundary_when_query_is_one_past_lower() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;

        // Edge case: min_query == lower_attestation + 1
        // Query block 11 → lower attestation at 10, upper at 20
        // In build_proof_from_roots, take_while(b.n() < 11) yields zero blocks
        // so lower_endpoint_digest falls back to the attestation digest at block 10.
        let chain = svc.chain_state(2).unwrap();
        let bounds = svc.get_attestation_boundaries(chain, 11, 11).await;
        assert!(bounds.is_some());
        let (lower, lower_digest, upper, upper_digest) = bounds.unwrap();
        assert_eq!(lower, 10);
        assert_eq!(upper, 20);
        // The digest should be the mock attestation's digest at block 10
        assert_ne!(lower_digest, H256::zero(), "digest should be non-zero");
        // The upper digest should be the mock attestation's digest at block 20
        assert_ne!(
            upper_digest,
            H256::zero(),
            "upper digest should be non-zero"
        );
    }

    #[tokio::test]
    async fn attestation_fallback_to_checkpoint_when_no_attestation_bounds() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;

        // Query beyond attestation range (mock attestations go up to 1000)
        // Should fail attestation lookup but succeed with checkpoint lookup
        let chain = svc.chain_state(2).unwrap();
        let att_bounds = svc.get_attestation_boundaries(chain, 1005, 1005).await;
        assert!(att_bounds.is_none(), "no attestation bounds beyond range");

        // Checkpoint at 1000 exists as lower, but no upper checkpoint
        let cp_bounds = svc.get_checkpoint_boundaries(chain, 1005, 1005).await;
        assert!(
            cp_bounds.is_none(),
            "no checkpoint upper bound beyond range"
        );
    }

    #[tokio::test]
    async fn revert_caches_truncates_entries_above_height() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let att = chain.attestation_cache.read().await;

        assert!(
            att.contains_key(&500),
            "attestation at 500 should exist before revert"
        );
        assert!(
            att.contains_key(&510),
            "attestation at 510 should exist before revert"
        );
        assert!(
            att.contains_key(&1000),
            "attestation at 1000 should exist before revert"
        );
        assert_eq!(*att.keys().next_back().unwrap(), 1000);
        drop(att);

        let cp = chain.checkpoint_cache.read().await;
        assert!(
            cp.contains_key(&500),
            "attestation at 500 should exist before revert"
        );
        assert!(
            cp.contains_key(&600),
            "attestation at 510 should exist before revert"
        );
        assert!(
            cp.contains_key(&1000),
            "attestation at 1000 should exist before revert"
        );
        assert_eq!(*cp.keys().next_back().unwrap(), 1000);
        drop(cp);

        // Mock populates attestations at 10, 20, ..., 1000
        // and checkpoints at 0, 100, 200, ..., 1000.
        // Revert to height 500: entries > 500 should be gone.
        svc.revert_caches(2, 500).await;

        // Every attestation at or below 500 survives; every one above is gone
        let att = chain.attestation_cache.read().await;
        for h in (10..=1000).step_by(10) {
            if h <= 500 {
                assert!(
                    att.contains_key(&h),
                    "attestation at {h} should survive revert"
                );
            } else {
                assert!(
                    !att.contains_key(&h),
                    "attestation at {h} should be removed by revert"
                );
            }
        }
        assert_eq!(*att.keys().next_back().unwrap(), 500);
        drop(att);

        // Every checkpoint at or below 500 survives; every one above is gone
        let cp = chain.checkpoint_cache.read().await;
        for h in (0..=1000).step_by(100) {
            if h <= 500 {
                assert!(
                    cp.contains_key(&h),
                    "checkpoint at {h} should survive revert"
                );
            } else {
                assert!(
                    !cp.contains_key(&h),
                    "checkpoint at {h} should be removed by revert"
                );
            }
        }
        assert_eq!(*cp.keys().next_back().unwrap(), 500);
        drop(cp);

        // Boundary lookups reflect the truncated state
        let bounds = svc.get_attestation_boundaries(chain, 505, 505).await;
        assert!(bounds.is_none(), "no upper attestation bound after revert");
    }

    #[tokio::test]
    async fn revert_caches_is_noop_for_unknown_chain() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;

        // Should not panic or affect chain 2's caches
        svc.revert_caches(999, 0).await;

        let chain = svc.chain_state(2).unwrap();
        let att = chain.attestation_cache.read().await;
        assert!(!att.is_empty());
    }

    #[tokio::test]
    async fn revert_caches_to_zero_clears_all() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        svc.revert_caches(2, 0).await;

        let att = chain.attestation_cache.read().await;
        // Attestations start at 10, so reverting to 0 clears everything
        assert!(att.is_empty(), "all attestations should be removed");
        drop(att);

        // Checkpoint at block 0 should survive (0 <= 0)
        let cp = chain.checkpoint_cache.read().await;
        assert_eq!(cp.len(), 1);
        assert!(cp.contains_key(&0));
    }

    /// `CcRpcProvider` that returns a different set from
    /// `get_stable_checkpoints_for_chain` than from
    /// `get_checkpoints_for_chain`. Models an in-flight chain reversion where
    /// the on-chain bucket-granular gate (`Pallet::checkpoint_if_stable`)
    /// would filter out some heights still physically present in raw storage.
    struct ReversionInFlightCcProvider {
        delegate: Arc<continuity::mocks::MockCcRpcProvider>,
        stable_block_numbers: std::collections::BTreeSet<u64>,
    }

    #[async_trait]
    impl continuity::rpc::CcRpcProvider for ReversionInFlightCcProvider {
        async fn get_attestations_for_chain(
            &self,
            chain_key: u64,
        ) -> Result<Vec<attestor_primitives::SignedAttestation<H256, cc_client::AccountId32>>>
        {
            self.delegate.get_attestations_for_chain(chain_key).await
        }
        async fn get_last_checkpoint(
            &self,
            chain_key: u64,
        ) -> Result<Option<attestor_primitives::AttestationCheckpoint>> {
            self.delegate.get_last_checkpoint(chain_key).await
        }
        async fn get_checkpoints_for_chain(
            &self,
            chain_key: u64,
        ) -> Result<Vec<attestor_primitives::AttestationCheckpoint>> {
            // Unfiltered: includes stale post-revert digests. Startup must NOT
            // call this path; if it does, the test fails because the stale
            // entries end up in the cache.
            self.delegate.get_checkpoints_for_chain(chain_key).await
        }
        async fn get_stable_checkpoints_for_chain(
            &self,
            chain_key: u64,
        ) -> Result<Vec<attestor_primitives::AttestationCheckpoint>> {
            // Filtered: the on-chain gate has dropped stale-pivot digests.
            let all = self.delegate.get_checkpoints_for_chain(chain_key).await?;
            Ok(all
                .into_iter()
                .filter(|cp| self.stable_block_numbers.contains(&cp.block_number))
                .collect())
        }
        async fn get_checkpoint_by_height(
            &self,
            chain_key: u64,
            block_number: u64,
        ) -> Result<Option<attestor_primitives::AttestationCheckpoint>> {
            self.delegate
                .get_checkpoint_by_height(chain_key, block_number)
                .await
        }
        async fn get_attestation_chain_genesis_block_number(&self, chain_key: u64) -> Result<u64> {
            self.delegate
                .get_attestation_chain_genesis_block_number(chain_key)
                .await
        }
        async fn fetch_last_digest(&self, chain_key: u64) -> Result<Option<H256>> {
            self.delegate.fetch_last_digest(chain_key).await
        }
        async fn get_attestation_by_digest(
            &self,
            chain_key: u64,
            digest: H256,
        ) -> Result<Option<attestor_primitives::SignedAttestation<H256, cc_client::AccountId32>>>
        {
            self.delegate
                .get_attestation_by_digest(chain_key, digest)
                .await
        }
        async fn get_attestation_interval(&self, chain_key: u64) -> Result<Option<u64>> {
            self.delegate.get_attestation_interval(chain_key).await
        }
        async fn get_checkpoint_interval(&self, chain_key: u64) -> Result<Option<u64>> {
            self.delegate.get_checkpoint_interval(chain_key).await
        }
    }

    #[tokio::test]
    async fn startup_uses_stable_checkpoints_and_drops_stale_pivots() {
        // Regression for the prover-API companion to the on-chain
        // `checkpoint_if_stable` gate: when the prover API boots while a
        // chain reversion is in flight, the checkpoint cache must be seeded
        // from the gated read so we never cache a stale post-revert digest.
        //
        // The mock produces checkpoints at 0, 100, 200, ..., 1000. We model
        // a reversion where heights > 500 still live in raw storage but the
        // on-chain gate has dropped them. The stable set therefore contains
        // only {0, 100, ..., 500}; the prover API cache must match exactly.
        let chain_key = 2;
        let (mock_cc, eth_provider) = continuity::mocks::make_mock_providers(chain_key);
        let stable: std::collections::BTreeSet<u64> = (0..=5).map(|i| i * 100).collect();
        let cc_provider = Arc::new(ReversionInFlightCcProvider {
            delegate: mock_cc,
            stable_block_numbers: stable.clone(),
        });

        let builder = Arc::new(ContinuityBuilder::new_with_providers(
            mock_config(chain_key),
            cc_provider,
            eth_provider,
        ));
        let svc = ContinuityService::new(vec![builder], NoopMetrics::new(), 10, 1_000)
            .await
            .expect("service init should succeed");

        let chain = svc.chain_state(chain_key).unwrap();
        let cp = chain.checkpoint_cache.read().await;

        let cached: std::collections::BTreeSet<u64> = cp.keys().copied().collect();
        assert_eq!(
            cached, stable,
            "checkpoint cache must contain exactly the stable set from the gated read; \
             stale post-revert digests must not be present"
        );
        for stale_height in [600u64, 700, 800, 900, 1000] {
            assert!(
                !cp.contains_key(&stale_height),
                "stale checkpoint at {stale_height} must not be cached on startup during a reversion",
            );
        }
    }

    #[tokio::test]
    async fn batch_span_exceeding_limit_is_rejected() {
        // Set a tight max_batch_span of 50 blocks
        let svc = make_service_with_batch_span(Arc::new(FoundEthProvider), 50).await;
        let chain = svc.chain_state(2).unwrap();

        // Blocks 100 and 200 are 100 apart, which exceeds the 50-block limit
        let queries = vec![
            ProofQuery {
                header_number: 100,
                tx_indexes: vec![],
            },
            ProofQuery {
                header_number: 200,
                tx_indexes: vec![],
            },
        ];
        let err = svc
            .get_proof_batch(chain, &queries)
            .await
            .expect_err("should reject span > max_batch_span");
        assert!(
            matches!(
                err,
                ServiceError::BatchSpanTooLarge {
                    span: 100,
                    max_span: 50,
                    ..
                }
            ),
            "expected BatchSpanTooLarge, got {err:?}"
        );
        assert!(!err.retriable());
        assert_eq!(err.code(), "BatchSpanTooLarge");
    }

    #[tokio::test]
    async fn batch_span_within_limit_is_accepted() {
        // Set max_batch_span of 50 blocks
        let svc = make_service_with_batch_span(Arc::new(FoundEthProvider), 50).await;
        let chain = svc.chain_state(2).unwrap();

        // Blocks 100 and 140 are 40 apart, which is within the 50-block limit.
        // The request will still fail (boundary lookup / RPC) but NOT with BatchSpanTooLarge.
        let queries = vec![
            ProofQuery {
                header_number: 100,
                tx_indexes: vec![],
            },
            ProofQuery {
                header_number: 140,
                tx_indexes: vec![],
            },
        ];
        let result = svc.get_proof_batch(chain, &queries).await;
        // It may fail for other reasons (mock doesn't serve real blocks) but
        // it must NOT fail with BatchSpanTooLarge.
        if let Err(err) = result {
            assert!(
                !matches!(err, ServiceError::BatchSpanTooLarge { .. }),
                "span within limit should not be rejected, got {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn single_block_batch_always_passes_span_check() {
        // Even with max_batch_span = 0, a single-block batch has span 0 and should pass.
        let svc = make_service_with_batch_span(Arc::new(FoundEthProvider), 0).await;
        let chain = svc.chain_state(2).unwrap();

        let queries = vec![ProofQuery {
            header_number: 100,
            tx_indexes: vec![],
        }];
        let result = svc.get_proof_batch(chain, &queries).await;
        if let Err(err) = result {
            assert!(
                !matches!(err, ServiceError::BatchSpanTooLarge { .. }),
                "single-block batch should never fail span check, got {err:?}"
            );
        }
    }

    // ---------------------------------------------------------------------
    // Regression tests for the "stale attestation cache" bug:
    // CheckpointReached on chain removes the constituent attestations, but the
    // proof-gen-api cache used to keep them, so proofs for blocks older than
    // the latest checkpoint were anchored on attestation digests the on-chain
    // verifier could no longer recognize.
    // ---------------------------------------------------------------------

    #[tokio::test]
    async fn prune_attestations_at_or_below_drops_consumed_entries() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        // Sanity: cache is seeded with attestations 10, 20, ..., 1000.
        {
            let att = chain.attestation_cache.read().await;
            assert!(att.contains_key(&500));
            assert!(att.contains_key(&510));
        }

        svc.prune_attestations_at_or_below(2, 500).await;

        let att = chain.attestation_cache.read().await;
        for h in (10..=1000).step_by(10) {
            let key = h as u64;
            if key <= 500 {
                assert!(
                    !att.contains_key(&key),
                    "attestation at {key} should be pruned (<=500)"
                );
            } else {
                assert!(
                    att.contains_key(&key),
                    "attestation at {key} should survive (>500)"
                );
            }
        }
    }

    #[tokio::test]
    async fn prune_attestations_at_or_below_does_not_touch_checkpoint_cache() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let cp_before: Vec<u64> = chain
            .checkpoint_cache
            .read()
            .await
            .keys()
            .copied()
            .collect();

        svc.prune_attestations_at_or_below(2, 500).await;

        let cp_after: Vec<u64> = chain
            .checkpoint_cache
            .read()
            .await
            .keys()
            .copied()
            .collect();
        assert_eq!(cp_before, cp_after, "checkpoint cache must not change");
    }

    #[tokio::test]
    async fn prune_attestations_at_or_below_preserves_cache_when_nothing_to_prune() {
        // Regression: BTreeMap::split_off mutates in place to hold the lower
        // half. A guard that only reassigns when `removed > 0` would silently
        // wipe the entire cache here (every entry is above the split key).
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let before: Vec<u64> = chain
            .attestation_cache
            .read()
            .await
            .keys()
            .copied()
            .collect();
        assert!(!before.is_empty(), "precondition: cache is seeded");

        // Prune at a height strictly below every entry (mock attestations
        // start at 10, so height=5 means nothing should be removed).
        svc.prune_attestations_at_or_below(2, 5).await;

        let after: Vec<u64> = chain
            .attestation_cache
            .read()
            .await
            .keys()
            .copied()
            .collect();
        assert_eq!(
            before, after,
            "cache must be untouched when no entry is at-or-below the prune height"
        );
    }

    #[tokio::test]
    async fn prune_attestations_at_or_below_is_noop_for_unknown_chain() {
        let svc = make_service(Arc::new(FoundEthProvider)).await;

        // Should not panic, should not affect chain 2.
        svc.prune_attestations_at_or_below(999, 500).await;

        let chain = svc.chain_state(2).unwrap();
        let att = chain.attestation_cache.read().await;
        assert!(att.contains_key(&500), "chain 2 cache must be untouched");
    }

    // ---------------------------------------------------------------------
    // Regression tests for the split-bracket bug:
    //
    // After a checkpoint at block H lands, `prune_attestations_at_or_below`
    // drops every attestation `<= H`. A query for the very first attestation
    // strictly above H then leaves the attestation cache with only the upper
    // half (the queried block itself, no entry below) and the checkpoint cache
    // with only the lower half (the checkpoint at H, no entry at or above
    // until the next checkpoint lands). Each cache alone fails to bracket the
    // query; mixing one half from each succeeds and is sound because both
    // caches store on-chain digests for the same digest sequence.
    // ---------------------------------------------------------------------

    /// Reset both caches to a known minimal state that mirrors the
    /// production split-bracket scenario:
    ///   attestation_cache = { upper_block: <digest> }
    ///   checkpoint_cache  = { lower_block: <digest> }
    /// where `lower_block < upper_block` and neither cache has the other half.
    async fn install_split_bracket(
        chain: &Arc<ChainState>,
        lower_checkpoint: u64,
        lower_digest: H256,
        upper_attestation: u64,
        upper_digest: H256,
    ) {
        let mut att = chain.attestation_cache.write().await;
        att.clear();
        att.insert(upper_attestation, upper_digest);
        drop(att);

        let mut cp = chain.checkpoint_cache.write().await;
        cp.clear();
        cp.insert(lower_checkpoint, lower_digest);
    }

    #[tokio::test]
    async fn split_bracket_each_cache_alone_fails() {
        // Sanity check: confirm that the production split-bracket shape
        // genuinely defeats both single-cache lookups. If this ever starts
        // passing on its own, the mixed-bracket fallback would be unreachable
        // and the regression test below would no longer cover what it claims.
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let lower = 10_797_800_u64;
        let upper = 10_797_810_u64;
        let lower_digest = H256::from_low_u64_be(0xA1);
        let upper_digest = H256::from_low_u64_be(0xB2);
        install_split_bracket(chain, lower, lower_digest, upper, upper_digest).await;

        let att_only = svc.get_attestation_boundaries(chain, upper, upper).await;
        assert!(
            att_only.is_none(),
            "attestation cache alone must not bracket the query (no lower half)"
        );

        let cp_only = svc.get_checkpoint_boundaries(chain, upper, upper).await;
        assert!(
            cp_only.is_none(),
            "checkpoint cache alone must not bracket the query (no upper half)"
        );
    }

    #[tokio::test]
    async fn split_bracket_mixed_lookup_combines_cp_lower_with_att_upper() {
        // Reproduces the dump shape from the production failure: query block
        // 10_797_810 sits exactly at the smallest attestation cache key and
        // exactly at the smallest cache key strictly above the latest
        // checkpoint. Each cache holds only one half; the mixed lookup must
        // combine `cp_lower` with `att_upper` and return both on-chain
        // digests untouched so the proof builder can anchor the digest chain.
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let lower = 10_797_800_u64;
        let upper = 10_797_810_u64;
        let lower_digest = H256::from_low_u64_be(0xA1);
        let upper_digest = H256::from_low_u64_be(0xB2);
        install_split_bracket(chain, lower, lower_digest, upper, upper_digest).await;

        let bounds = svc.get_mixed_boundaries(chain, upper, upper).await;
        assert!(
            bounds.is_some(),
            "mixed lookup should bracket the query when each cache holds one half"
        );
        let (lo_block, lo_digest, up_block, up_digest) = bounds.unwrap();
        assert_eq!(lo_block, lower, "lower must come from the checkpoint cache");
        assert_eq!(
            lo_digest, lower_digest,
            "lower digest must be the on-chain checkpoint digest, untouched"
        );
        assert_eq!(
            up_block, upper,
            "upper must come from the attestation cache"
        );
        assert_eq!(
            up_digest, upper_digest,
            "upper digest must be the on-chain attestation digest, untouched"
        );
    }

    #[tokio::test]
    async fn split_bracket_mixed_lookup_falls_back_to_att_lower_with_cp_upper() {
        // Symmetric case: the only half in the attestation cache is the
        // lower, and the only half in the checkpoint cache is the upper.
        // Mixed lookup must still bracket the query.
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let lower = 10_797_805_u64;
        let upper = 10_797_900_u64;
        let lower_digest = H256::from_low_u64_be(0xC3);
        let upper_digest = H256::from_low_u64_be(0xD4);

        // attestation cache holds the lower; checkpoint cache holds the upper.
        let mut att = chain.attestation_cache.write().await;
        att.clear();
        att.insert(lower, lower_digest);
        drop(att);

        let mut cp = chain.checkpoint_cache.write().await;
        cp.clear();
        cp.insert(upper, upper_digest);
        drop(cp);

        let query = 10_797_810_u64;
        let bounds = svc.get_mixed_boundaries(chain, query, query).await;
        assert!(bounds.is_some(), "mixed lookup should bracket the query");
        let (lo_block, lo_digest, up_block, up_digest) = bounds.unwrap();
        assert_eq!(lo_block, lower);
        assert_eq!(lo_digest, lower_digest);
        assert_eq!(up_block, upper);
        assert_eq!(up_digest, upper_digest);
    }

    #[tokio::test]
    async fn split_bracket_mixed_lookup_prefers_attestation_upper_when_both_have_upper() {
        // When both caches happen to expose an upper, the attestation upper
        // is tighter and should win. Lower comes from the checkpoint cache
        // because that path is tried first.
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let cp_lower = 10_797_800_u64;
        let att_upper = 10_797_810_u64;
        let cp_upper = 10_797_900_u64;
        let cp_lower_digest = H256::from_low_u64_be(0xE5);
        let att_upper_digest = H256::from_low_u64_be(0xF6);
        let cp_upper_digest = H256::from_low_u64_be(0x07);

        let mut att = chain.attestation_cache.write().await;
        att.clear();
        att.insert(att_upper, att_upper_digest);
        drop(att);

        let mut cp = chain.checkpoint_cache.write().await;
        cp.clear();
        cp.insert(cp_lower, cp_lower_digest);
        cp.insert(cp_upper, cp_upper_digest);
        drop(cp);

        let bounds = svc.get_mixed_boundaries(chain, att_upper, att_upper).await;
        let (lo_block, _, up_block, up_digest) =
            bounds.expect("mixed lookup should resolve when both halves are present");
        assert_eq!(lo_block, cp_lower);
        assert_eq!(up_block, att_upper, "tighter attestation upper must win");
        assert_eq!(up_digest, att_upper_digest);
    }

    #[tokio::test]
    async fn split_bracket_mixed_lookup_returns_none_when_neither_half_pair_exists() {
        // No lower in either cache → mixed lookup must still return None.
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let upper = 10_797_810_u64;
        let upper_digest = H256::from_low_u64_be(0xB2);
        let mut att = chain.attestation_cache.write().await;
        att.clear();
        att.insert(upper, upper_digest);
        drop(att);

        let mut cp = chain.checkpoint_cache.write().await;
        cp.clear();
        drop(cp);

        let bounds = svc.get_mixed_boundaries(chain, upper, upper).await;
        assert!(
            bounds.is_none(),
            "mixed lookup must not invent a lower anchor when neither cache has one"
        );
    }

    #[tokio::test]
    async fn split_bracket_mixed_lookup_does_not_mutate_caches() {
        // The fallback is read-only: it must not insert, remove, or otherwise
        // touch the cache contents. A future refactor that, for example,
        // "backfills" the missing half across caches would silently shift the
        // pruning invariant and is explicitly out of scope here.
        let svc = make_service(Arc::new(FoundEthProvider)).await;
        let chain = svc.chain_state(2).unwrap();

        let lower = 10_797_800_u64;
        let upper = 10_797_810_u64;
        install_split_bracket(
            chain,
            lower,
            H256::from_low_u64_be(1),
            upper,
            H256::from_low_u64_be(2),
        )
        .await;

        let att_before: Vec<(u64, H256)> = chain
            .attestation_cache
            .read()
            .await
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();
        let cp_before: Vec<(u64, H256)> = chain
            .checkpoint_cache
            .read()
            .await
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();

        let _ = svc.get_mixed_boundaries(chain, upper, upper).await;

        let att_after: Vec<(u64, H256)> = chain
            .attestation_cache
            .read()
            .await
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();
        let cp_after: Vec<(u64, H256)> = chain
            .checkpoint_cache
            .read()
            .await
            .iter()
            .map(|(&k, &v)| (k, v))
            .collect();

        assert_eq!(att_before, att_after, "attestation cache must be untouched");
        assert_eq!(cp_before, cp_after, "checkpoint cache must be untouched");
    }
}
