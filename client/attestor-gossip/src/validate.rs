use std::fmt::{Debug, Display};

use sc_client_api::{Backend, BlockBackend};
use sc_network::NetworkPeers;
use sp_api::ProvideRuntimeApi;
use sp_consensus::SyncOracle;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};

use log::{debug, error, info, warn};
use parity_scale_codec::Codec;

use attestor_primitives::{
    api::AttestorApi,
    block::Block,
    bls::{Bls, BlsSerialize, CryptoScheme, PublicKey},
};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;

use crate::{
    communication::{Attestation, Error},
    Client, HashFor, Worker, LOG_TARGET,
};

impl<B: BlockT, RA: ProvideRuntimeApi<B>, BE, C, AccountId, S, N>
    Worker<B, RA, BE, C, AccountId, S, N>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: BabeApi<B>,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RA::Api: SupportedChainsApi<B>,
    RA::Api: RandomnessPalletApi<B>,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    <<B as BlockT>::Header as HeaderT>::Number: Into<u64>,
    S: SyncOracle,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
    N: NetworkPeers,
{
    pub fn validate_attestation(
        &mut self,
        block_hash: B::Hash,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let chain_key = attestation.chain_key();
        let header_number = attestation.header_number();

        self.verify_vrf(block_hash, attestation)?;
        debug!(target: LOG_TARGET, "📝 VRF output verified successfully");

        // Check if the attestation bls signature is valid
        if !self.verify_bls_signature(block_hash, attestation)? {
            error!("Invalid BLS signature");
            return Err(Error::InvalidBlsSignature);
        }

        let runtime = self.runtime.runtime_api();

        let exists = runtime.contains_digest(block_hash, chain_key, attestation.digest())?;
        if exists {
            debug!(target: LOG_TARGET, "📝 Attestation already exists, discarding");
            return Err(Error::AttestationExists);
        }

        let is_chain_supported =
            runtime.is_chain_supported(block_hash, attestation.attestation_data.chain_key)?;

        if !is_chain_supported {
            debug!(target: LOG_TARGET, "📝 Chain is not supported, attestation rejected");
            return Err(Error::ChainNotSupported);
        }

        // Every attestation must have a continuity proof
        // except for the first attestation in the chain
        if attestation.continuity_proof.is_empty() && header_number != 0 {
            return Err(Error::InvalidAttestationContinuityProof);
        }

        // Get last digest, either checkpoint or last attestation
        let mut last_block_digest = match runtime.last_digest(block_hash, chain_key)? {
            Some(digest) => digest,
            None => {
                // If no last digest is found, assume genesis block
                info!(target: LOG_TARGET, "📝 No last digest or checkpoint found for block hash: {block_hash:?}, assuming genesis block");
                H256::zero()
            }
        };

        // Validate the attestation's previous digest,
        match attestation.prev_digest() {
            Some(digest) => {
                if digest.is_zero() && !last_block_digest.is_zero() {
                    error!(target: LOG_TARGET, "❌ Attestation has a zero prev digest and we don't have a finalized attestation yet");
                    return Err(Error::InvalidAttestationContinuityProof);
                }
            }
            None => {
                if !last_block_digest.is_zero() {
                    error!(target: LOG_TARGET, "❌ Attestation has no prev digest and we don't have a finalized attestation yet");
                    return Err(Error::InvalidAttestationContinuityProof);
                }
            }
        }

        info!(target: LOG_TARGET, "📝 Checking Continuity proof, length: {:?}, round: {:?}, last_block_digest: {:?}", attestation.continuity_proof.len(), attestation.round(), last_block_digest);

        // Validate the prev digest of the attestation against the head of the continuity proof
        if let Some(attestation_head) = attestation.continuity_proof.head() {
            let block: Block = attestation_head.clone().into();
            let block_digest = H256::from_slice(&block.digest.to_bytes_be());

            if block_digest != attestation.prev_digest().unwrap_or_default() {
                error!(target: LOG_TARGET, "❌ Continuity proof head digest mismatch, expected {:?}, got {:?}", attestation.prev_digest().unwrap_or_default(), block_digest);
                return Err(Error::InvalidAttestationContinuityProof);
            }
        }

        // Check if the tail's prev_digest of the fragment matches the last finalized attestation
        // Otherwise check if we actually have the digest in storage, it could be that the last finalized attestation from attestation view is not the last finalized attestation in storage
        // This could happen if the attestation view is lagging behind
        if let Some(tail) = attestation.continuity_proof.tail() {
            let block: Block = tail.clone().into();
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());
            if block_prev_digest != last_block_digest {
                // Check if we have the block_prev_digest in storage
                let exists = runtime.contains_digest(block_hash, chain_key, block_prev_digest)?;
                if !exists {
                    error!(target: LOG_TARGET, "❌ Continuity proof tail prev digest mismatch, expected {last_block_digest:?}, got {block_prev_digest:?}, and we don't have it in storage");
                    return Err(Error::InvalidAttestationContinuityProof);
                } else {
                    last_block_digest = block_prev_digest;
                    debug!(target: LOG_TARGET, "📝 Continuity proof tail prev digest mismatch, expected {last_block_digest:?}, got {block_prev_digest:?}, but we have it in storage, continuing");
                }
            }
        }

        for serializable in attestation.continuity_proof.get_blocks_ref().clone() {
            let block: Block = serializable.into();

            let block_digest = H256::from_slice(&block.digest.to_bytes_be());
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());

            // Check if the last block digest matches the previous digest of the current block
            // This to ensure that the continuity proof is valid
            if last_block_digest == block_prev_digest {
                debug!(target: LOG_TARGET, "📝 Continuity proof continues with block {block:?}");
            } else {
                error!(target: LOG_TARGET, "❌ Continuity proof invalid, expected {last_block_digest:?}, got {block_prev_digest:?}, block: {block:?}");
                return Err(Error::InvalidAttestationContinuityProof);
            }
            // Update the last block digest to the current block's digest
            last_block_digest = block_digest;
        }

        info!(target: LOG_TARGET, "✅ Attestation continuity proof & signature are valid.");
        Ok(())
    }

    /// Verify the signatures on the attestation
    /// It checks the BLS signature with the public key it provided when it registered on chain
    fn verify_bls_signature(
        &self,
        block_hash: B::Hash,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<bool, Error> {
        // Check if the BLS signature is correct
        let runtime = self.runtime.runtime_api();
        let bls_pubkey = runtime
            .attestor_bls_pubkey(
                block_hash,
                attestation.attestation_data.chain_key,
                &attestation.attestor,
            )?
            .ok_or(Error::NotAnAttestor(attestation.attestor_id()))?;

        let bls_pubkey = PublicKey::from_bytes(&bls_pubkey[..]).map_err(|e| {
            error!(target: LOG_TARGET, "📝 invalid bls signature: {e:?}");
            Error::InvalidBlsSignature
        })?;

        let msg = attestation.attestation_data.serialize();
        let bls_valid =
            <Bls as CryptoScheme>::verify(&bls_pubkey, &attestation.signature_bls, &msg);

        Ok(bls_valid)
    }

    /// Verify the VRF output for an attestation.
    /// This checks if the attestor that submitted this attestations vrf output is correct
    /// Correct being, that it signed the babe's VRF output from Two epochs ago & that the attestor is eligible to submit an attestation
    fn verify_vrf(
        &mut self,
        at: B::Hash,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        debug!(target: LOG_TARGET, "📝 Verifying VRF output for attestation");
        let chain_key = attestation.chain_key();
        let header_number = attestation.header_number();

        let attestor_id = attestation.attestor_id();

        // Get randomness from the attestation
        let attestation_epoch = attestation.proof_of_inclusion.epoch;
        let runtime = self.runtime.runtime_api();
        let randomness = runtime.randomness_by_epoch_id(at, attestation_epoch)?;

        // Here we verify the proof of inclusion
        // based on the round config
        // Get round config at the attestation epoch
        let round_config = self.get_or_create_round_config(at, chain_key)?;

        let is_included = vrf::verify_proof_of_inclusion(
            round_config.committee_set_size.into(),
            round_config.target_sample_size.into(),
            &randomness,
            &attestation.proof_of_inclusion,
            &attestor_id,
            header_number,
        )?;

        if !is_included {
            warn!(target: LOG_TARGET, "📝 Attestor {attestor_id:?} not eligible");
            return Err(Error::AttestorNotEligible(attestor_id));
        }

        debug!(target: LOG_TARGET, "📝 Attestor {attestor_id:?} selected ✅");
        Ok(())
    }
}
