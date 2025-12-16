use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;

use super::EndsInAttestation;
use anyhow::{anyhow, Result};
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::AccountId32;
use sp_core::H256;
use tracing::{debug, info};

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

        // Check if attestations cover the range we need
        let has_attestation_lower = attestations
            .iter()
            .any(|a| a.attestation.header_number < required_before);
        let has_attestation_upper = attestations
            .iter()
            .any(|a| a.attestation.header_number > max_query);

        let is_at_attestation = attestations
            .iter()
            .any(|a| a.attestation.header_number == min_query);

        // Only fetch checkpoints if we need them (attestations don't fully cover the range)
        // This avoids expensive RPC calls when attestations are sufficient
        let checkpoints: Option<Vec<AttestationCheckpoint>> =
            if !has_attestation_lower || !has_attestation_upper {
                self.cc_provider
                    .get_checkpoints_for_chain(self.config.chain_key)
                    .await
                    .ok()
            } else {
                None
            };

        // Check if query is at a checkpoint height (only if we fetched checkpoints)
        let is_at_checkpoint = checkpoints
            .as_ref()
            .map(|cps| cps.iter().any(|c| c.block_number == min_query))
            .unwrap_or(false);

        if is_at_attestation || is_at_checkpoint {
            info!(
                query_type = if is_at_attestation {
                    "attestation"
                } else {
                    "checkpoint"
                },
                is_at_attestation, is_at_checkpoint, "Query is at consensus point height"
            );
        }

        // Find lower bound
        let lower_info = self
            .find_lower_bound(required_before, attestations, checkpoints.as_deref())
            .await?;

        // Find upper bound
        let (upper_info, ends_in_attestation) = self.find_upper_bound(
            max_query,
            is_at_attestation,
            is_at_checkpoint,
            attestations,
            checkpoints.as_deref(),
        )?;

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
        // Find the best attestation STRICTLY BEFORE required_before
        // We need strictly before because when building continuity, we use the lower bound's
        // digest as prev_digest for computing the NEXT block's digest. If we used an exact
        // match at required_before, we'd incorrectly use that block's digest as its own prev_digest.
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number < required_before)
            .max_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            });

        // Find checkpoint lower bound if checkpoints were provided
        let checkpoint_lower = checkpoints.and_then(|cps| {
            cps.iter()
                .filter(|c| c.block_number < required_before)
                .max_by_key(|c| c.block_number)
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                })
        });

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
        is_at_attestation: bool,
        is_at_checkpoint: bool,
        attestations: &[SignedAttestation<H256, AccountId32>],
        checkpoints: Option<&[AttestationCheckpoint]>,
    ) -> Result<(Option<AttestationInfo>, EndsInAttestation)> {
        if is_at_attestation {
            // Query is at an attestation height - use that attestation as upper bound
            Ok((
                attestations
                    .iter()
                    .find(|a| a.attestation.header_number == max_query)
                    .map(|a| AttestationInfo {
                        block_number: a.attestation.header_number,
                        digest: a.attestation.digest(),
                    }),
                EndsInAttestation::True,
            ))
        } else if is_at_checkpoint {
            // Query is at a checkpoint height - use that checkpoint as upper bound
            Ok((
                checkpoints.and_then(|cps| {
                    cps.iter()
                        .find(|c| c.block_number == max_query)
                        .map(|c| AttestationInfo {
                            block_number: c.block_number,
                            digest: c.digest,
                        })
                }),
                EndsInAttestation::False,
            ))
        } else {
            // Find next consensus point after max_query
            let attestation_upper = attestations
                .iter()
                .filter(|a| a.attestation.header_number > max_query)
                .min_by_key(|a| a.attestation.header_number)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                });

            // Find checkpoint upper bound if checkpoints were provided
            let checkpoint_upper = checkpoints.and_then(|cps| {
                cps.iter()
                    .filter(|c| c.block_number > max_query)
                    .min_by_key(|c| c.block_number)
                    .map(|c| AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
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
}
