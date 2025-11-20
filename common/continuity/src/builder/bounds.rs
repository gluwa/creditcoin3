use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;

use anyhow::{anyhow, Result};
use attestor_primitives::SignedAttestation;
use cc_client::AccountId32;
use sp_core::H256;

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
    ) -> Result<(AttestationInfo, Option<AttestationInfo>)> {
        // Check if query is exactly at an attestation height
        let is_at_attestation = attestations
            .iter()
            .any(|a| a.attestation.header_number == min_query);

        // Check if query is at checkpoint height (lazy check - only queries last checkpoint)
        let is_at_checkpoint = self
            .check_if_at_checkpoint_height(min_query)
            .await?
            .is_some();

        if is_at_attestation || is_at_checkpoint {
            println!(
                "Query is at {} height (attestation: {}, checkpoint: {})",
                if is_at_attestation {
                    "attestation"
                } else {
                    "checkpoint"
                },
                is_at_attestation,
                is_at_checkpoint
            );
            println!("Fetching previous attestation/checkpoint to build continuity proof...");
        }

        // Find lower bound: closest attestation or checkpoint before min_query
        // Requires continuity to start at queryHeight - 1, so we need consensus point before that
        // If query is at an attestation/checkpoint height, we need the previous one
        let required_before = min_query.saturating_sub(1);

        // Find best lower bound from attestations
        let attestation_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number < required_before)
            .max_by_key(|a| a.attestation.header_number);

        // Only fetch checkpoints if:
        // 1. Query is at checkpoint height (need previous checkpoint)
        // 2. No attestation found before required_before (need to check checkpoints)
        let checkpoint_lower = if is_at_checkpoint || attestation_lower.is_none() {
            // Use max_needed to get checkpoints BEFORE required_before (not min_needed which filters them out!)
            let checkpoints = self
                .fetch_checkpoints_smart(Some(required_before), None)
                .await?;

            checkpoints
                .into_iter()
                .filter(|c| c.block_number < required_before)
                .max_by_key(|c| c.block_number)
        } else {
            None
        };

        // Choose the closest one (highest block number) before required_before
        let lower_info = match (attestation_lower, checkpoint_lower) {
            (Some(a), Some(c)) => {
                if a.attestation.header_number > c.block_number {
                    AttestationInfo {
                        block_number: a.attestation.header_number,
                        digest: a.attestation.digest(),
                    }
                } else {
                    AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
                    }
                }
            }
            (Some(a), None) => AttestationInfo {
                block_number: a.attestation.header_number,
                digest: a.attestation.digest(),
            },
            (None, Some(c)) => AttestationInfo {
                block_number: c.block_number,
                digest: c.digest,
            },
            (None, None) => {
                // Provide helpful error message with suggestions
                let error_msg = format!(
                    "No attestation or checkpoint found before block {required_before} (query height: {min_query}).\n\
                    The continuity proof requires a consensus point (attestation or checkpoint) \
                    before block {required_before} to start the continuity chain.\n\n\
                    Possible solutions:\n\
                    1. Ensure checkpoints are imported for chain_key {} using import_checkpoints\n\
                    2. Wait for an attestation at a block before the query height\n\
                    3. Query a block height that has an earlier attestation/checkpoint",
                    self.config.chain_key
                );
                return Err(anyhow!(error_msg));
            }
        };

        // Find upper bound: if query is at an attestation/checkpoint height, use that as upper bound
        // Otherwise, find the next attestation/checkpoint after max_query
        let upper_info = if is_at_attestation {
            // Query is at an attestation height - use that attestation as upper bound
            attestations
                .iter()
                .find(|a| a.attestation.header_number == max_query)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                })
        } else if is_at_checkpoint {
            // Query is at a checkpoint height - use that checkpoint as upper bound
            let checkpoints = self.fetch_checkpoints_smart(None, None).await?;
            checkpoints
                .into_iter()
                .find(|c| c.block_number == max_query)
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                })
        } else {
            // Query is not at an attestation/checkpoint height - find next one after max_query
            let attestation_upper = attestations
                .iter()
                .filter(|a| a.attestation.header_number > max_query)
                .min_by_key(|a| a.attestation.header_number)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                });

            // Only fetch checkpoints for upper bound if no attestation found after max_query
            let checkpoint_upper = if attestation_upper.is_none() {
                let checkpoints = self.fetch_checkpoints_smart(Some(max_query), None).await?;
                checkpoints
                    .into_iter()
                    .filter(|c| c.block_number > max_query)
                    .min_by_key(|c| c.block_number)
                    .map(|c| AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
                    })
            } else {
                None
            };

            // Choose the closest one (lowest block number) after max_query
            match (attestation_upper, checkpoint_upper) {
                (Some(a), Some(c)) => {
                    if a.block_number < c.block_number {
                        Some(a)
                    } else {
                        Some(c)
                    }
                }
                (Some(a), None) => Some(a),
                (None, Some(c)) => Some(c),
                (None, None) => None,
            }
        };

        // Log the bounds for debugging
        println!(
            "Attestation bounds: lower={} upper={}",
            lower_info.block_number,
            upper_info
                .as_ref()
                .map(|u| u.block_number)
                .unwrap_or(max_query + 10)
        );

        Ok((lower_info, upper_info))
    }
}
