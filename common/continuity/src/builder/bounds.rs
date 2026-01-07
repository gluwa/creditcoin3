use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;

use super::EndsInAttestation;
use anyhow::{anyhow, Result};
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;
use tracing::debug;

impl ContinuityBuilder {
    /// Find optimal attestation bounds for the query range
    ///
    /// Handles special case: when query is at an attestation or checkpoint height,
    /// we need to fetch the previous attestation/checkpoint to compute the continuity proof.
    pub async fn find_attestation_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<(AttestationInfo, Option<AttestationInfo>, EndsInAttestation)> {
        let required_before = min_query.saturating_sub(1);

        // If we have a lower attestation, then all checkpoints should be at
        // IMPORTANT: Always fetch checkpoints FIRST before attestations to avoid race condition.
        // If a checkpoint exists, the corresponding attestation may have been evicted from the
        // retention buffer. Checkpoints are permanent and won't be evicted.
        let checkpoints: Option<Vec<AttestationCheckpoint>> = self
            .cc_provider
            .get_checkpoints_for_chain(self.config.chain_key)
            .await
            .ok();

        // Find lower bound
        let lower_info = self
            .find_lower_bound(required_before, attestations, checkpoints.as_deref())
            .await?;

        // Find upper bound
        let (upper_info, ends_in_attestation) =
            self.find_upper_bound(max_query, attestations, checkpoints.as_deref())?;

        debug!(
            lower_bound = lower_info.block_number,
            upper_bound = upper_info
                .as_ref()
                .map(|u| u.block_number)
                .unwrap_or(max_query + 10),
            "Attestation bounds determined"
        );

        Ok((lower_info, upper_info, ends_in_attestation))
    }

    async fn find_lower_bound(
        &self,
        required_before: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
        checkpoints: Option<&[AttestationCheckpoint]>,
    ) -> Result<AttestationInfo> {
        // IMPORTANT: Check checkpoints FIRST before attestations to avoid race condition.
        // Checkpoints are permanent and won't be evicted, while attestations may be evicted
        // from the retention buffer after a checkpoint is created.

        // Find checkpoint lower bound if checkpoints were provided
        let checkpoint_lower = if let Some(cps) = checkpoints {
            cps.iter()
                .filter(|c| c.block_number <= required_before)
                .max_by_key(|c| c.block_number)
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                    prev_digest: None, // Checkpoints don't have prev_digest, will be computed in build.rs if needed
                })
        } else {
            None
        };

        // Find the best attestation at or before required_before
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number <= required_before)
            .max_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
                prev_digest: a.prev_digest(),
            });

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
                    chain_key = self.config.chain_key,
                    "No attestation or checkpoint found before required block. \
                    Ensure checkpoints are imported or wait for attestations."
                );
                Err(anyhow!(
                    "No consensus point found before block {required_before} (query height: {query_height})"
                ))
            }
        }
    }

    // Additionally returns a bool indicating whether the upper bound is an attestation `ends_in_attestation`
    // IMPORTANT: Check checkpoints FIRST before attestations to avoid race condition.
    fn find_upper_bound(
        &self,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
        checkpoints: Option<&[AttestationCheckpoint]>,
    ) -> Result<(Option<AttestationInfo>, EndsInAttestation)> {
        // Find checkpoint upper bound first (checkpoints are permanent, won't be evicted)
        let checkpoint_upper = checkpoints.and_then(|cps| {
            cps.iter()
                .filter(|c| c.block_number >= max_query)
                .min_by_key(|c| c.block_number)
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                    prev_digest: None,
                })
        });

        // Find next attestation after max_query (may be evicted after checkpoint creation)
        let attestation_upper = attestations
            .iter()
            .filter(|a| a.attestation.header_number >= max_query)
            .min_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
                prev_digest: a.prev_digest(),
            });

        // Choose the closest one (lowest block number)
        // Prefer checkpoint if both exist at the same block number (checkpoint is permanent)
        Ok(match (checkpoint_upper, attestation_upper) {
            (Some(c), Some(a)) => {
                if c.block_number <= a.block_number {
                    (Some(c), EndsInAttestation::False)
                } else {
                    (Some(a), EndsInAttestation::True)
                }
            }
            (Some(c), None) => (Some(c), EndsInAttestation::False),
            (None, Some(a)) => (Some(a), EndsInAttestation::True),
            (None, None) => (None, EndsInAttestation::False),
        })
    }
}
