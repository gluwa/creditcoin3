//! CC3 chain-based bounds finding
//!
//! This module finds attestation/checkpoint bounds by querying the CC3 chain directly.
//! This is slower than using the indexer but works when no indexer is available.

use super::BoundsFinder;
use crate::builder::ContinuityBuilder;
use crate::errors::ContinuityError;
use async_trait::async_trait;
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::AccountId32;
use indexer_client::AttestationWithProof;
use sp_core::H256;
use tracing::{debug, info};

/// Bounds finder that queries the CC3 chain directly.
pub struct Cc3BoundsFinder<'a> {
    builder: &'a ContinuityBuilder,
}

impl<'a> Cc3BoundsFinder<'a> {
    pub fn new(builder: &'a ContinuityBuilder) -> Self {
        Self { builder }
    }
}

#[async_trait]
impl<'a> BoundsFinder for Cc3BoundsFinder<'a> {
    async fn find_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        current_block: Option<u64>,
    ) -> Result<(AttestationWithProof, AttestationWithProof), ContinuityError> {
        info!(
            chain_key = self.builder.config.chain_key,
            "Fetching attestations from CC3 chain (no indexer)"
        );

        // Fetch attestations from CC3 chain (slow!)
        let attestations = self
            .builder
            .fetch_attestations()
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?;

        info!(
            attestation_count = attestations.len(),
            "Fetched attestations from CC3 chain"
        );

        if attestations.is_empty() {
            return Err(ContinuityError::NoAttestations(
                self.builder.config.chain_key,
            ));
        }

        // Find attestation bounds (handles queries at attestation/checkpoint heights)
        // Checkpoints are fetched lazily only when needed
        let (lower, upper) = self
            .find_attestation_bounds(min_query, max_query, &attestations)
            .await
            .map_err(|e| ContinuityError::Rpc(e.to_string()))?;

        // If no upper bound exists (block not yet attested), predict the next attestation
        let upper = match upper {
            Some(u) => u,
            None => {
                let predicted = self.builder.predict_next_attestation(max_query).await?;
                self.builder.validate_predicted_upper_bound(
                    predicted.block_number,
                    max_query,
                    current_block,
                )?;

                info!(
                    max_query,
                    predicted_upper = predicted.block_number,
                    "No attestation found after query block - using predicted upper bound"
                );
                predicted
            }
        };

        Ok((lower, upper))
    }
}

impl<'a> Cc3BoundsFinder<'a> {
    /// Find optimal attestation bounds for the query range
    ///
    /// Handles special case: when query is at an attestation or checkpoint height,
    /// we need to fetch the previous attestation/checkpoint to compute the continuity proof.
    async fn find_attestation_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<(AttestationWithProof, Option<AttestationWithProof>), ContinuityError> {
        let required_before = min_query.saturating_sub(1);

        // IMPORTANT: Always fetch checkpoints FIRST before attestations to avoid race condition.
        // If a checkpoint exists, the corresponding attestation may have been evicted from the
        // retention buffer. Checkpoints are permanent and won't be evicted.
        let checkpoints: Option<Vec<AttestationCheckpoint>> = self
            .builder
            .cc_provider
            .get_checkpoints_for_chain(self.builder.config.chain_key)
            .await
            .ok();

        // Find lower bound
        let lower_info = self
            .find_lower_bound(required_before, attestations, checkpoints.as_deref())
            .await?;

        // Find upper bound
        let upper_info = self.find_upper_bound(max_query, attestations, checkpoints.as_deref())?;

        debug!(
            lower_bound = lower_info.block_number,
            upper_bound = upper_info
                .as_ref()
                .map(|u| u.block_number)
                .unwrap_or(max_query + 10),
            "Attestation bounds determined"
        );

        Ok((lower_info, upper_info))
    }

    async fn find_lower_bound(
        &self,
        required_before: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
        checkpoints: Option<&[AttestationCheckpoint]>,
    ) -> Result<AttestationWithProof, ContinuityError> {
        // IMPORTANT: Check checkpoints FIRST before attestations to avoid race condition.
        // Checkpoints are permanent and won't be evicted, while attestations may be evicted
        // from the retention buffer after a checkpoint is created.

        // Find checkpoint lower bound if checkpoints were provided
        let checkpoint_lower = if let Some(cps) = checkpoints {
            cps.iter()
                .filter(|c| c.block_number <= required_before)
                .max_by_key(|c| c.block_number)
                .map(AttestationWithProof::from_checkpoint)
        } else {
            None
        };

        // Find the best attestation at or before required_before
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number <= required_before)
            .max_by_key(|a| a.attestation.header_number)
            .map(AttestationWithProof::from_signed_attestation);

        // Choose the closest one (highest block number)
        // Prefer checkpoint if both exist at the same block number (checkpoint is permanent)
        match (checkpoint_lower, attestation_lower) {
            (Some(c), Some(a)) => Ok(if c.block_number >= a.block_number {
                c
            } else {
                a
            }),
            (Some(c), None) => Ok(c),
            (None, Some(a)) => Ok(a),
            (None, None) => {
                let query_height = required_before + 1;
                tracing::error!(
                    required_before,
                    query_height,
                    chain_key = self.builder.config.chain_key,
                    "No attestation or checkpoint found before required block. \
                    Ensure checkpoints are imported or wait for attestations."
                );
                Err(ContinuityError::Rpc(format!(
                    "No consensus point found before block {required_before} (query height: {query_height})"
                )))
            }
        }
    }

    /// IMPORTANT: Check checkpoints FIRST before attestations to avoid race condition.
    fn find_upper_bound(
        &self,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
        checkpoints: Option<&[AttestationCheckpoint]>,
    ) -> Result<Option<AttestationWithProof>, ContinuityError> {
        // Find checkpoint upper bound first (checkpoints are permanent, won't be evicted)
        let checkpoint_upper = checkpoints.and_then(|cps| {
            cps.iter()
                .filter(|c| c.block_number >= max_query)
                .min_by_key(|c| c.block_number)
                .map(AttestationWithProof::from_checkpoint)
        });

        // Find next attestation after max_query (may be evicted after checkpoint creation)
        let attestation_upper = attestations
            .iter()
            .filter(|a| a.attestation.header_number >= max_query)
            .min_by_key(|a| a.attestation.header_number)
            .map(AttestationWithProof::from_signed_attestation);

        // Choose the closest one (lowest block number)
        // Prefer checkpoint if both exist at the same block number (checkpoint is permanent)
        Ok(match (checkpoint_upper, attestation_upper) {
            (Some(c), Some(a)) => {
                if c.block_number <= a.block_number {
                    Some(c)
                } else {
                    Some(a)
                }
            }
            (Some(c), None) => Some(c),
            (None, Some(a)) => Some(a),
            (None, None) => None,
        })
    }
}
