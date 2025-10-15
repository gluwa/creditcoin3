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
        let attestation_genesis = AttestationChainGenesisBlockNumber::<T>::get(chain_key);

        // Every attestation must have a continuity proof
        // except for the first attestation in the chain
        if attestation.continuity_proof.is_empty() && header_number != attestation_genesis {
            return Err(Error::<T>::EmptyContinuityProof.into());
        }

        // Get last digest, either checkpoint or last attestation
        let last_block_digest = Self::last_digest(chain_key);
        info!(
            "📝 Last finalized attestation digest for chain_key {chain_key:?}: {last_block_digest:?}"
        );

        info!(
            "📝 Validating attestation continuity for attestation: chain_key: {chain_key:?}, header_number: {header_number}, digest: {:?}, prev_digest: {:?}, continuity_proof length: {}",
            attestation.digest(),
            attestation.prev_digest(),
            attestation.continuity_proof.len()
        );
        // Validate the attestation's previous digest,
        match attestation.prev_digest() {
            Some(digest) => {
                if digest.is_zero() && last_block_digest.is_some() {
                    error!("❌ Attestation has a zero prev digest and we don't have a finalized attestation yet");
                    return Err(Error::<T>::InvalidAttestationPrevDigest.into());
                }
            }
            None => {
                if last_block_digest.is_some() {
                    error!(
                        "❌ Attestation has no prev digest but we have a finalized attestation yet"
                    );
                    return Err(Error::<T>::InvalidAttestationPrevDigest.into());
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
                return Err(Error::<T>::InvalidAttestationContinuityProofHead.into());
            }
        }

        // Unwrap or default the last block digest
        let mut last_block_digest = last_block_digest.unwrap_or_default();

        // Check if the tail's prev_digest of the fragment matches the last finalized attestation
        // Otherwise check if we actually have the digest in storage, it could be that the last finalized attestation from attestation view is not the last finalized attestation in storage
        // This could happen if the attestation view is lagging behind
        if let Some(tail) = attestation.continuity_proof.tail() {
            let block: Block = tail.clone().into();
            info!("📝 Checking continuity proof tail: {block:?}");
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());

            // In almost all cases, the tail's prev_digest should match one of the previously finalized attestations
            if let Some(prev_attestation) = Self::get(chain_key, block_prev_digest) {
                if prev_attestation.header_number() != block.block_number - 1 {
                    error!("❌ Continuity proof tail prev digest points to an attestation with header number {}, but expected {}", attestation.header_number(), block.block_number - 1);
                    return Err(Error::<T>::InvalidAttestationContinuityProofTail.into());
                }
            }

            let attestation_genesis = AttestationChainGenesisBlockNumber::<T>::get(chain_key);
            if block_prev_digest.is_zero() && block.block_number != attestation_genesis {
                error!("❌ Continuity proof tail prev digest is zero, but block number is not genesis ({attestation_genesis})");
                return Err(Error::<T>::InvalidAttestationContinuityProofBlockGenesis.into());
            }

            last_block_digest = block_prev_digest;
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

            // Check if the last block digest matches the previous digest of the current block
            // This to ensure that the continuity proof is valid
            if last_block_digest != block_prev_digest {
                return Err(Error::<T>::InvalidAttestationContinuityProofBlock.into());
            }

            debug!("📝 Continuity proof continues with block {block:?}");
            // Update the last block digest to the current block's digest
            last_block_digest = block_digest;
        }

        debug!("✅ Attestation continuity proof & signature are valid.");
        Ok(())
    }
}
