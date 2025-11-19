use super::ContinuityBuilder;
use crate::attestation::AttestationInfo;

use anyhow::{anyhow, Result};
use attestor_primitives::SignedAttestation;
use cc_client::AccountId32;
use sp_core::H256;

impl ContinuityBuilder {
    pub async fn find_attestation_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        attestations: &[SignedAttestation<H256, AccountId32>],
    ) -> Result<(AttestationInfo, Option<AttestationInfo>)> {
        let is_at_attestation = attestations
            .iter()
            .any(|a| a.attestation.header_number == min_query);

        let is_at_checkpoint = self
            .check_if_at_checkpoint_height(min_query)
            .await?
            .is_some();

        let required_before = min_query.saturating_sub(1);

        // Lower bound ---------------------------------------------------------
        let att_lower = attestations
            .iter()
            .filter(|a| a.attestation.header_number < required_before)
            .max_by_key(|a| a.attestation.header_number);

        let cp_lower = if is_at_checkpoint || att_lower.is_none() {
            let cps = self
                .fetch_checkpoints_smart(Some(required_before), None)
                .await?;
            cps.into_iter()
                .filter(|c| c.block_number < required_before)
                .max_by_key(|c| c.block_number)
        } else {
            None
        };

        let lower = match (att_lower, cp_lower) {
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
                return Err(anyhow!(
                    "No attestation or checkpoint found before required height {required_before}"
                ))
            }
        };

        // Upper bound ---------------------------------------------------------
        let upper = if is_at_attestation {
            attestations
                .iter()
                .find(|a| a.attestation.header_number == max_query)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                })
        } else if is_at_checkpoint {
            let cps = self.fetch_checkpoints_smart(None, None).await?;
            cps.into_iter()
                .find(|c| c.block_number == max_query)
                .map(|c| AttestationInfo {
                    block_number: c.block_number,
                    digest: c.digest,
                })
        } else {
            let att_upper = attestations
                .iter()
                .filter(|a| a.attestation.header_number > max_query)
                .min_by_key(|a| a.attestation.header_number)
                .map(|a| AttestationInfo {
                    block_number: a.attestation.header_number,
                    digest: a.attestation.digest(),
                });

            let cp_upper = if att_upper.is_none() {
                let cps = self.fetch_checkpoints_smart(Some(max_query), None).await?;
                cps.into_iter()
                    .filter(|c| c.block_number > max_query)
                    .min_by_key(|c| c.block_number)
                    .map(|c| AttestationInfo {
                        block_number: c.block_number,
                        digest: c.digest,
                    })
            } else {
                None
            };

            match (att_upper, cp_upper) {
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

        Ok((lower, upper))
    }
}
