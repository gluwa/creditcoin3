use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;

use super::EndsInAttestation;
use anyhow::{anyhow, Result};
use attestor_primitives::SignedAttestation;
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
        let is_at_attestation = attestations
            .iter()
            .any(|a| a.attestation.header_number == min_query);
        let is_at_checkpoint = self
            .check_if_at_checkpoint_height(min_query)
            .await?
            .is_some();

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

        let required_before = min_query.saturating_sub(1);

        // Find lower bound
        let lower_info = self
            .find_lower_bound(required_before, is_at_checkpoint, attestations)
            .await?;

        // Find upper bound
        let (upper_info, ends_in_attestation) = self
            .find_upper_bound(max_query, is_at_attestation, is_at_checkpoint, attestations)
            .await?;

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
        is_at_checkpoint: bool,
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<AttestationInfo> {
        // First, check if there's an attestation or checkpoint exactly at required_before
        // If so, we can use it directly (e.g., genesis checkpoint when querying block after genesis)
        let exact_attestation = attestations
            .iter()
            .find(|a| a.attestation.header_number == required_before)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            });

        let exact_checkpoint = if exact_attestation.is_none() {
            self.cc_provider
                .get_checkpoint_by_height(self.config.chain_key, required_before)
                .await
                .ok()
                .flatten()
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                })
        } else {
            None
        };

        // If we found an exact match, use it
        if let Some(exact) = exact_attestation.or(exact_checkpoint) {
            return Ok(exact);
        }

        // Otherwise, find the best one before required_before (exclusive)
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number < required_before)
            .max_by_key(|a| a.attestation.header_number)
            .map(|a| AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            });

        // Find best checkpoint if needed
        let checkpoint_lower = if is_at_checkpoint || attestation_lower.is_none() {
            self.find_checkpoint_lower(required_before).await?
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

    async fn find_checkpoint_lower(&self, required_before: u64) -> Result<Option<AttestationInfo>> {
        // Optimization: Check last checkpoint first (cheap single query)
        // If it's before required_before, we're done without fetching all checkpoints
        if let Ok(Some(last_cp)) = self
            .cc_provider
            .get_last_checkpoint(self.config.chain_key)
            .await
        {
            if last_cp.block_number < required_before {
                return Ok(Some(AttestationInfo {
                    block_number: last_cp.block_number,
                    digest: last_cp.digest,
                }));
            }
        }

        // Fallback to fetching all and filtering (only if last checkpoint doesn't satisfy)
        let checkpoints = self
            .fetch_checkpoints_smart(Some(required_before), None)
            .await?;

        Ok(checkpoints
            .into_iter()
            .filter(|c| c.block_number < required_before)
            .max_by_key(|c| c.block_number)
            .map(|c| AttestationInfo {
                block_number: c.block_number,
                digest: c.digest,
            }))
    }

    // Additionally returns a bool indicating whether the upper bound is an attestation `ends_in_attestation`
    async fn find_upper_bound(
        &self,
        max_query: u64,
        is_at_attestation: bool,
        is_at_checkpoint: bool,
        attestations: &[SignedAttestation<H256, AccountId32>],
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
                self.cc_provider
                    .get_checkpoint_by_height(self.config.chain_key, max_query)
                    .await
                    .ok()
                    .flatten()
                    .map(|c| AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
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

            let checkpoint_upper = if attestation_upper.is_none() {
                // Optimization: Check last checkpoint first (cheap single query)
                // If it's > max_query, it might be what we need, but we need to verify it's the minimum
                // by checking if there are any checkpoints between max_query and last_checkpoint
                let last_checkpoint = self
                    .cc_provider
                    .get_last_checkpoint(self.config.chain_key)
                    .await
                    .ok()
                    .flatten();

                if let Some(last_cp) = last_checkpoint {
                    if last_cp.block_number > max_query {
                        // Last checkpoint is after max_query, but we need the minimum
                        // So we still need to fetch all to find the minimum
                        let checkpoints =
                            self.fetch_checkpoints_smart(Some(max_query), None).await?;
                        checkpoints
                            .into_iter()
                            .filter(|c| c.block_number > max_query)
                            .min_by_key(|c| c.block_number)
                            .map(|c| AttestationInfo {
                                block_number: c.block_number,
                                digest: c.digest,
                            })
                    } else {
                        // Last checkpoint is <= max_query, so no checkpoint after max_query exists
                        None
                    }
                } else {
                    // No checkpoints at all
                    None
                }
            } else {
                None
            };

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
