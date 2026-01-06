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
        // even lower block heights. So we don't need to use checkpoints at all.
        let has_attestation_lower = attestations
            .iter()
            .any(|a| a.attestation.header_number <= required_before);

        // Only fetch checkpoints if we need them (attestations don't fully cover the range)
        // This avoids expensive RPC calls when attestations are sufficient
        let checkpoints: Option<Vec<AttestationCheckpoint>> = if !has_attestation_lower {
            self.cc_provider
                .get_checkpoints_for_chain(self.config.chain_key)
                .await
                .ok()
        } else {
            None
        };

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
        // Find the best attestation at or before required_before
        // We allow exact matches (non-strict) because when lower.block_number == required_start,
        // the build logic in build_and_trim_continuity handles this case correctly by using
        // the lower bound's digest appropriately. The continuity chain construction ensures
        // proper digest chaining even when the lower bound matches required_start.
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number <= required_before)
            .max_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
                prev_digest: a.prev_digest(),
            });

        // Find checkpoint lower bound if checkpoints were provided
        // If checkpoint is at required_before (query height = checkpoint height + 1),
        // we need to find the previous checkpoint and build continuity to get the digest
        // of block (checkpoint_height - 1) to use as lower_endpoint_digest
        let checkpoint_lower = if let Some(cps) = checkpoints {
            cps.iter()
                .filter(|c| c.block_number <= required_before)
                .max_by_key(|c| c.block_number)
                .map(|c| {
                    // If checkpoint is exactly at required_before, prev_digest will be computed
                    // in build.rs by finding the previous checkpoint and building continuity
                    AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
                        prev_digest: None, // Will be computed in build.rs if needed
                    }
                })
        } else {
            None
        };

        // Choose the closest one (highest block number)
        match (attestation_lower, checkpoint_lower) {
            (Some(a), Some(c)) => Ok(if a.block_number > c.block_number {
                a
            } else {
                c
            }),
            (Some(a), None) => Ok(a),
            (None, Some(c)) => Ok(c),
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
    fn find_upper_bound(
        &self,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
        checkpoints: Option<&[AttestationCheckpoint]>,
    ) -> Result<(Option<AttestationInfo>, EndsInAttestation)> {
        // Find next consensus point after max_query
        let attestation_upper = attestations
            .iter()
            .filter(|a| a.attestation.header_number >= max_query)
            .min_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
                prev_digest: a.prev_digest(),
            });

        // Find checkpoint upper bound if checkpoints were provided
        // Note: checkpoints don't have prev_digest, so it will be None
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

        Ok(match (attestation_upper, checkpoint_upper) {
            (Some(a), Some(c)) => {
                if a.block_number < c.block_number {
                    (Some(a), EndsInAttestation::True)
                } else {
                    (Some(c), EndsInAttestation::False)
                }
            }
            (Some(a), None) => (Some(a), EndsInAttestation::True),
            (None, Some(c)) => (Some(c), EndsInAttestation::False),
            (None, None) => (None, EndsInAttestation::False),
        })
    }
}
