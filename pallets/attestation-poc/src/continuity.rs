use attestor_primitives::{block::Block, SignedAttestation};
use frame_support::pallet_prelude::*;
use log::{debug, error, info};
use sp_core::H256;

use super::pallet::*;

/// PALLET CALL IMPLS ///
impl<T: Config> Pallet<T> {
    pub fn validate_attestation_continuity(
        attestation: &SignedAttestation<T::Hash, T::AccountId>,
    ) -> DispatchResult {
        let chain_key = attestation.chain_key();
        let header_number = attestation.header_number();

        // Every attestation must have a continuity proof
        // except for the first attestation in the chain
        if attestation.continuity_proof.is_empty() && header_number != 0 {
            return Err(Error::<T>::InvalidAttestationContinuityProof.into());
        }

        // Get last digest, either checkpoint or last attestation
        let mut last_block_digest = match Self::last_digest(chain_key) {
            Some(digest) => digest,
            None => {
                // If no last digest is found, assume genesis block
                info!("📝 No last digest or checkpoint found assuming genesis block");
                H256::zero()
            }
        };

        // Validate the attestation's previous digest,
        match attestation.prev_digest() {
            Some(digest) => {
                if digest.is_zero() && !last_block_digest.is_zero() {
                    error!("❌ Attestation has a zero prev digest and we don't have a finalized attestation yet");
                    return Err(Error::<T>::InvalidAttestationContinuityProof.into());
                }
            }
            None => {
                if !last_block_digest.is_zero() {
                    error!(
                        "❌ Attestation has no prev digest but we have a finalized attestation yet"
                    );
                    return Err(Error::<T>::InvalidAttestationContinuityProof.into());
                }
            }
        }

        info!(
            "📝 Checking Continuity proof, length: {:?}, round: {:?}, last_block_digest: {:?}",
            attestation.continuity_proof.len(),
            attestation.round(),
            last_block_digest
        );

        // Validate the prev digest of the attestation against the head of the continuity proof
        if let Some(attestation_head) = attestation.continuity_proof.head() {
            let block: Block = attestation_head.clone().into();
            let block_digest = H256::from_slice(&block.digest.to_bytes_be());

            if block_digest != attestation.prev_digest().unwrap_or_default() {
                error!(
                    "❌ Continuity proof head digest mismatch, expected {:?}, got {:?}",
                    attestation.prev_digest().unwrap_or_default(),
                    block_digest
                );
                return Err(Error::<T>::InvalidAttestationContinuityProof.into());
            }
        }

        // Check if the tail's prev_digest of the fragment matches the last finalized attestation
        // Otherwise check if we actually have the digest in storage, it could be that the last finalized attestation from attestation view is not the last finalized attestation in storage
        // This could happen if the attestation view is lagging behind
        if let Some(tail) = attestation.continuity_proof.tail() {
            let block: Block = tail.clone().into();
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());
            if block_prev_digest != last_block_digest {
                // Check if we have the last_block_digest in storage
                let exists = Self::contains_digest(chain_key, last_block_digest);
                if !exists {
                    error!("❌ Continuity proof tail prev digest mismatch, expected {last_block_digest:?}, got {block_prev_digest:?}, and we don't have it in storage");
                    return Err(Error::<T>::InvalidAttestationContinuityProof.into());
                } else {
                    last_block_digest = block_prev_digest;
                    debug!("📝 Continuity proof tail prev digest mismatch, expected {last_block_digest:?}, got {block_prev_digest:?}, but we have it in storage, continuing");
                }
            }
        }

        for serializable in attestation.continuity_proof.get_blocks_ref().clone() {
            let block: Block = serializable.clone().into();

            let block_digest = H256::from_slice(&block.digest.to_bytes_be());
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());

            info!(
                "📝 Checking block number: {}, block_digest: {:?}, block_root: {:?} block_prev_digest: {:?}",
                block.block_number,
                block_digest,
                block.root,
                block_prev_digest,
            );

            // Check if the last block digest matches the previous digest of the current block
            // This to ensure that the continuity proof is valid
            if last_block_digest == block_prev_digest {
                debug!("📝 Continuity proof continues with block {block:?}");
            } else {
                error!("❌ Continuity proof invalid, expected {last_block_digest:?}, got {block_prev_digest:?}, block: {block:?}");
                return Err(Error::<T>::InvalidAttestationContinuityProof.into());
            }
            // Update the last block digest to the current block's digest
            last_block_digest = block_digest;
        }

        info!("✅ Attestation continuity proof & signature are valid.");
        Ok(())
    }
}
