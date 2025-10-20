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
        debug!("🔍 Validating attestation continuity...");
        let chain_key = attestation.chain_key();
        let attestation_header_number = attestation.header_number();
        let attestation_genesis = AttestationChainGenesisBlockNumber::<T>::get(chain_key);

        debug!(
            "📝 Validating attestation continuity for attestation: chain_key: {chain_key:?}, attestation_header_number: {attestation_header_number}, digest: {:?}, prev_digest: {:?}, continuity_proof length: {}",
            attestation.digest(),
            attestation.prev_digest(),
            attestation.continuity_proof.len()
        );
        // GENESIS: must not have a prev digest; continuity proof can be empty
        if attestation_header_number == attestation_genesis {
            ensure!(
                attestation.prev_digest().is_none(),
                Error::<T>::InvalidAttestationPrevDigest
            );
            info!("✅ Genesis attestation continuity proof is valid.");
            return Ok(());
        }

        // NON-GENESIS: prev_digest must exist and be non-zero
        let attestation_prev_digest = attestation
            .prev_digest()
            .ok_or(Error::<T>::InvalidAttestationPrevDigest)?;
        ensure!(
            !attestation_prev_digest.is_zero(),
            Error::<T>::InvalidAttestationPrevDigest
        );

        // Get last digest, either checkpoint or last attestation
        let mut last_finalized_digest = Self::last_digest(chain_key).ok_or_else(|| {
            error!("❌ No finalized attestation found for chain_key {chain_key:?}");
            Error::<T>::NoFinalizedAttestation
        })?;

        // If the attestation's prev digest matches the last finalized digest
        // We need no continuity proof, the attestation links directly.
        // We still need to check the header number continuity
        if attestation_prev_digest == last_finalized_digest
            && attestation.continuity_proof.is_empty()
        {
            let prev_attestation = Self::get(chain_key, attestation_prev_digest).ok_or_else(|| {
                error!("❌ Previous attestation with digest {attestation_prev_digest:?} not found in storage");
                Error::<T>::InvalidAttestationPrevDigest
            })?;

            ensure!(
                attestation_header_number == prev_attestation.header_number() + 1,
                Error::<T>::InvalidAttestationPrevDigest
            );

            info!("✅ Attestation continuity proof is valid (prev digest matches last finalized digest).");
            return Ok(());
        }

        // NON-GENESIS: must have a continuity proof
        ensure!(
            !attestation.continuity_proof.is_empty(),
            Error::<T>::EmptyContinuityProof
        );

        debug!(
            "📝 Checking Continuity proof, length: {:?}, round: {:?}, last_finalized_digest: {:?}",
            attestation.continuity_proof.len(),
            attestation.round(),
            last_finalized_digest
        );

        // Validate the prev digest of the attestation against the head of the continuity proof
        if let Some(attestation_head) = attestation.continuity_proof.head() {
            let block: Block = (*attestation_head).clone().into();
            let block_digest = H256::from_slice(&block.digest.to_bytes_be());
            ensure!(
                block_digest == attestation_prev_digest,
                Error::<T>::InvalidAttestationContinuityProofHead
            );
        }

        // Check if the tail's prev_digest of the fragment matches the last finalized attestation
        // Otherwise check if we actually have the digest in storage, it could be that the last finalized attestation from attestation view is not the last finalized attestation in storage
        // This could happen if the attestation view is lagging behind
        if let Some(tail) = attestation.continuity_proof.tail() {
            let block: Block = tail.clone().into();
            debug!("📝 Checking continuity proof tail: {block:?}");
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());

            // In almost all cases, the tail's prev_digest should match one of the previously finalized attestations
            if let Some(prev_attestation) = Self::get(chain_key, block_prev_digest) {
                if prev_attestation.header_number() != block.block_number - 1 {
                    error!("❌ Continuity proof tail prev digest points to an attestation with header number {}, but expected {}", prev_attestation.header_number(), block.block_number - 1);
                    return Err(Error::<T>::InvalidAttestationContinuityProofTail.into());
                }
            } else {
                error!("❌ Continuity proof tail prev digest {block_prev_digest:?} does not point to any known finalized attestation");
                return Err(Error::<T>::InvalidAttestationContinuityProofTail.into());
            }

            // Overwrite the last block digest to the tail's prev_digest
            // In order to validate the continuity proof from tail to head
            last_finalized_digest = block_prev_digest;
        }

        for serializable in attestation.continuity_proof.get_blocks_ref().clone() {
            let block: Block = serializable.clone().into();

            let block_digest = H256::from_slice(&block.digest.to_bytes_be());
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());

            debug!(
                "📝 Checking block number: {}, block_digest: {:?}, block_root: {:?} block_prev_digest: {:?}",
                block.block_number,
                block_digest,
                block.root,
                block_prev_digest,
            );

            // Link must continue exactly
            ensure!(
                last_finalized_digest == block_prev_digest,
                Error::<T>::InvalidAttestationContinuityProofBlock
            );

            debug!("📝 Continuity proof continues with block {block:?}");
            // Update the last block digest to the current block's digest
            last_finalized_digest = block_digest;
        }

        debug!("✅ Attestation continuity proof is valid.");
        Ok(())
    }
}
