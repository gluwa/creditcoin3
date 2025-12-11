use attestor_primitives::{block::Block, SignedAttestation};
use frame_support::pallet_prelude::*;
use log::{debug, error, info};

use super::pallet::*;

/// PALLET CALL IMPLS ///
impl<T: Config> Pallet<T> {
    pub fn validate_attestation_continuity(
        attestation: &SignedAttestation<T::Hash, T::AccountId>,
    ) -> DispatchResult {
        info!("🔍 Validating attestation continuity...");
        let chain_key = attestation.chain_key();
        let attestation_header_number = attestation.header_number();
        let attestation_genesis = AttestationChainGenesisBlockNumber::<T>::get(chain_key);

        info!(
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
        let last_finalized_digest = Self::last_digest(chain_key).ok_or_else(|| {
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

        // Get the tail block to determine the starting digest for reconstruction
        // The tail's prev_digest should point to a known finalized attestation
        let tail_block = attestation
            .continuity_proof
            .tail()
            .ok_or(Error::<T>::InvalidAttestationContinuityProofTail)?;
        let tail: Block = tail_block.clone().into();
        debug!("📝 Checking continuity proof tail: {tail:?}");
        let tail_prev_digest = tail.prev_digest;

        // Verify the tail's prev_digest points to a known finalized attestation
        if let Some(prev_attestation) = Self::get(chain_key, tail_prev_digest) {
            if prev_attestation.header_number() != tail.block_number - 1 {
                error!("❌ Continuity proof tail prev digest points to an attestation with header number {}, but expected {}", prev_attestation.header_number(), tail.block_number - 1);
                return Err(Error::<T>::InvalidAttestationContinuityProofTail.into());
            }
        } else {
            error!("❌ Continuity proof tail prev digest {tail_prev_digest:?} does not point to any known finalized attestation");
            return Err(Error::<T>::InvalidAttestationContinuityProofTail.into());
        }

        // Start reconstructing the digest chain from the tail's prev_digest
        // Continuity proof doesn't carry intermediate digests, so we reconstruct them iteratively
        let mut reconstructed_digest = tail_prev_digest;

        // Iterate through all blocks in the continuity proof, reconstructing digests from roots
        for serializable in attestation.continuity_proof.get_blocks_ref() {
            let block: Block = serializable.clone().into();

            debug!(
                "📝 Reconstructing digest for block number: {}, root: {:?}, prev_digest: {:?}",
                block.block_number, block.root, reconstructed_digest,
            );

            // Reconstruct the digest: digest[i] = hash(block_number, root[i], digest[i-1])
            // This matches what the attester computed: digest[i] = hash(digest[i-1], root[i])
            reconstructed_digest =
                Block::hash_payload(&block.block_number, &block.root, &reconstructed_digest);
        }

        // CRITICAL: Verify the final reconstructed digest matches the attestation's prev_digest
        // This ensures the continuity proof correctly links to the attestation and that all roots
        // were correctly bound in the digest chain
        ensure!(
            reconstructed_digest == attestation_prev_digest,
            Error::<T>::InvalidAttestationContinuityProofHead
        );

        debug!("✅ Attestation continuity proof is valid.");
        Ok(())
    }
}
