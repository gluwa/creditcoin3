use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use sp_api::ProvideRuntimeApi;
use sp_core::H256;
use sp_runtime::traits::Block as BlockT;

use log::{debug, error};
use parity_scale_codec::Codec;
use starknet_crypto::Felt;

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
        self.verify_bls_signature(block_hash, attestation)?;

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

        // Check that continuity from prior attestation is valid
        let mut last_block_digest: Option<Felt> = None;
        debug!(target: LOG_TARGET, "📝 Checking Continuity proof, length: {:?}", attestation.continuity_proof.len());
        for serializable in attestation.continuity_proof.get_blocks_ref().clone() {
            debug!(target: LOG_TARGET, "📝 Checking continuity proof for block {:?}", serializable);
            let block: Block = Block::try_from(serializable)
                .map_err(|_| Error::InvalidAttestationContinuityProof)?;

            let computed_digest = Block::hash_payload(&block.block_number.into(), &block.root);

            if computed_digest != block.digest {
                return Err(Error::InvalidAttestationContinuityProof);
            }

            // Check continuity
            match last_block_digest {
                Some(last_digest) /*if last_digest == block.prev_digest */ => {
                    last_block_digest = Some(block.digest);
                }
                Some(_) => return Err(Error::InvalidAttestationContinuityProof),
                None => last_block_digest = Some(block.digest),
            }
        }

        debug!(target: LOG_TARGET, "📝 Attestation is valid");
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
