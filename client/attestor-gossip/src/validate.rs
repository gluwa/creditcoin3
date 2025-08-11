use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use sp_api::ProvideRuntimeApi;
use sp_core::H256;
use sp_runtime::traits::Block as BlockT;

use log::{debug, error, info};
use parity_scale_codec::Codec;

use attestation_chain::block::Block;
use attestor_primitives::{
    api::AttestorApi,
    bls::{Bls, BlsSerialize, CryptoScheme, PublicKey},
};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;

use crate::{
    communication::{Attestation, Error},
    HashFor, LOG_TARGET,
};

pub struct AttestationValidator<B, AccountId, RA: ProvideRuntimeApi<B>>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    /// runtime api access
    pub runtime: Arc<RA>,

    phantom: std::marker::PhantomData<(B, AccountId)>,
}

impl<B, AccountId, RA: ProvideRuntimeApi<B>> AttestationValidator<B, AccountId, RA>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RA::Api: SupportedChainsApi<B>,
    RA::Api: RandomnessPalletApi<B>,
{
    pub fn new(runtime: Arc<RA>) -> Self {
        Self {
            runtime,
            phantom: std::marker::PhantomData,
        }
    }
}

impl<B, AccountId, RA: ProvideRuntimeApi<B>> AttestationValidator<B, AccountId, RA>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RA::Api: SupportedChainsApi<B>,
    RA::Api: RandomnessPalletApi<B>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    pub fn validate_attestation(
        &self,
        block_hash: B::Hash,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        let chain_key = attestation.chain_key();
        let header_number = attestation.header_number();

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
                info!(target: LOG_TARGET, "📝 No last digest or checkpoint found for block hash: {:?}, assuming genesis block", block_hash);
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
            let block: Block = Block::try_from(attestation_head.clone())
                .map_err(|_| Error::InvalidAttestationContinuityProof)?;
            let block_digest = H256::from_slice(&block.digest.to_bytes_be());

            if block_digest != attestation.prev_digest().unwrap_or_default() {
                error!(target: LOG_TARGET, "❌ Continuity proof head digest mismatch, expected {:?}, got {:?}", attestation.prev_digest().unwrap_or_default(), block_digest);
                return Err(Error::InvalidAttestationContinuityProof);
            }
        }

        for serializable in attestation.continuity_proof.get_blocks_ref().clone() {
            let block: Block = Block::try_from(serializable.clone())
                .map_err(|_| Error::InvalidAttestationContinuityProof)?;

            let block_digest = H256::from_slice(&block.digest.to_bytes_be());
            let block_prev_digest = H256::from_slice(&block.prev_digest.to_bytes_be());

            // Check if the last block digest matches the previous digest of the current block
            // This to ensure that the continuity proof is valid
            if last_block_digest == block_prev_digest {
                debug!(target: LOG_TARGET, "📝 Continuity proof continues with block {:?}", block);
            } else {
                error!(target: LOG_TARGET, "❌ Continuity proof invalid, expected {:?}, got {:?}, block: {:?}", last_block_digest, block_prev_digest, block);
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
            error!(target: LOG_TARGET, "📝 invalid bls signature: {:?}", e);
            Error::InvalidBlsSignature
        })?;

        let msg = attestation.attestation_data.serialize();
        let bls_valid =
            <Bls as CryptoScheme>::verify(&bls_pubkey, &attestation.signature_bls, &msg);

        Ok(bls_valid)
    }
}
