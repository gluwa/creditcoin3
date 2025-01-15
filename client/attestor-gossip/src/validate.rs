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

use attestation_chain::block::{Block, BlockSerializable};
use attestor_primitives::{
    api::AttestorApi,
    bls::{Bls, BlsSerialize, CryptoScheme, PublicKey},
    Round,
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
        round: Round,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<(), Error> {
        // Check if the attestation bls signature is valid
        self.verify_bls_signature(block_hash, attestation)?;

        let runtime = self.runtime.runtime_api();

        let exists = runtime.contains_digest(block_hash, round.0, attestation.digest())?;
        if exists {
            debug!(target: LOG_TARGET, "📝 Attestation already exists, discarding");
            return Err(Error::AttestationExists);
        }

        // Check that continuity from prior attestation is valid
        let flattened_proof = attestation
            .continuity_proof
            .iter()
            .flat_map(|frag| frag.get_blocks_ref().clone())
            .collect::<Vec<BlockSerializable>>();
        let mut last_block_digest: Option<Felt> = None;
        for serializable in flattened_proof {
            let block: Block = Block::try_from(serializable)
                .map_err(|_| Error::InvalidAttestationContinuityProof)?;

            let computed_digest =
                Block::hash_payload(&block.block_number.into(), &block.root, &block.prev_digest);

            if computed_digest != block.digest {
                return Err(Error::InvalidAttestationContinuityProof);
            }

            if let Some(last_digest) = last_block_digest {
                if last_digest == block.prev_digest {
                    last_block_digest = Some(block.digest);
                } else {
                    return Err(Error::InvalidAttestationContinuityProof);
                }
            } else {
                // The digest of the first block in our continutiy proof
                // should be identical to the prior attestation digest
                let last_digest = runtime
                    .last_digest(block_hash, attestation.attestation_data.chain_key)?
                    .ok_or(Error::InvalidAttestationContinuityProof)?;
                let last_digest_felt = Felt::from_dec_str(&hex::encode(last_digest)).unwrap();
                if block.digest == last_digest_felt {
                    last_block_digest = Some(block.digest);
                } else {
                    return Err(Error::InvalidAttestationContinuityProof);
                }
            }
        }

        let is_chain_supported =
            runtime.is_chain_supported(block_hash, attestation.attestation_data.chain_key)?;

        if !is_chain_supported {
            debug!(target: LOG_TARGET, "📝 Chain is not supported, attestation rejected");
            return Err(Error::ChainNotSupported);
        }

        debug!(target: LOG_TARGET, "📝 Attestation signature is valid");
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
