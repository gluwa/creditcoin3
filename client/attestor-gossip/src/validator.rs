use log::{error, info};
use parity_scale_codec::Codec;
use parity_scale_codec::Decode;
use randomness_primitives::api::RandomnessPalletApi;
use sc_client_api::{Backend, HeaderBackend};
use sc_network::PeerId;
use sc_network_gossip::{ValidationResult, Validator, ValidatorContext};
use sp_api::ProvideRuntimeApi;
use sp_core::{Pair, H256};
use sp_runtime::traits::Block as BlockT;
use std::fmt::Debug;
use std::fmt::Display;
use std::marker::PhantomData;
use std::sync::Arc;
use supported_chains_primitives::api::SupportedChainsApi;

use attestor_primitives::{
    api::AttestorApi,
    bls::{Bls, BlsSerialize, CryptoScheme, PublicKey},
};

use crate::{worker::votes_topic, HashFor, LOG_TARGET};

use super::{Action, Attestation, Error, Message};

pub struct AttestorGossipValidator<B, AccountId, RA: ProvideRuntimeApi<B>, BE>
where
    B: BlockT,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    BE: Backend<B>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    _phantom: PhantomData<B>,
    _phantom2: PhantomData<AccountId>,

    /// runtime api access
    pub runtime: Arc<RA>,

    /// Client Backend
    pub backend: Arc<BE>,
}

impl<B, AccountId, RA: ProvideRuntimeApi<B>, BE> AttestorGossipValidator<B, AccountId, RA, BE>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    RA::Api: SupportedChainsApi<B>,
    RA::Api: RandomnessPalletApi<B>,
    BE: Backend<B>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    pub fn new(runtime_api: Arc<RA>, backend: Arc<BE>) -> Self {
        Self {
            _phantom: Default::default(),
            _phantom2: Default::default(),
            runtime: runtime_api,
            backend,
        }
    }

    pub fn validate_attestation(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
        _sender: &PeerId,
    ) -> Result<Action<B::Hash>, Error> {
        let valid_sig = self.verify_signature(attestation)?;
        if !valid_sig {
            info!(target: LOG_TARGET, "📝 Attestation signature is invalid");
            return Err(Error::InvalidAttestationDataSignature);
        };

        let block_hash = self.backend.blockchain().info().best_hash;
        let runtime = self.runtime.runtime_api();
        let is_chain_supported =
            runtime.is_chain_supported(block_hash, attestation.attestation_data.chain_key)?;

        if !is_chain_supported {
            info!(target: LOG_TARGET, "📝 Chain is not supported, attestation rejected");
            return Err(Error::ChainNotSupported);
        }

        info!(target: LOG_TARGET, "📝 Attestation signature is valid");
        Ok(Action::Keep(votes_topic::<B>()))
    }

    /// Verify the signatures on the attestation
    /// It checks 2 parts, the signature created by the attestor's sr25519 key to verify he is actually owns that key
    /// Also checks the BLS signature with the public key it provided when it registered on chain
    /// If both check out, we can accept this attestation
    fn verify_signature(
        &self,
        attestation: &Attestation<HashFor<B>, AccountId>,
    ) -> Result<bool, Error> {
        let public_key = sp_core::sr25519::Public::from_raw(attestation.attestor.clone().into());

        let msg = attestation.attestation_data.serialize();

        let sr_valid = sp_core::sr25519::Pair::verify(&attestation.signature, &msg, &public_key);

        if !sr_valid {
            return Err(Error::InvalidSrSignature);
        }

        // Check if the BLS signature is correct
        let blockchain_info = self.backend.blockchain().info();
        let runtime = self.runtime.runtime_api();
        let bls_pubkey = runtime
            .attestor_bls_pubkey(
                blockchain_info.best_hash,
                attestation.attestation_data.chain_key,
                &attestation.attestor,
            )?
            .ok_or(Error::NotAnAttestor)?;

        let bls_pubkey = PublicKey::from_bytes(&bls_pubkey[..]).map_err(|e| {
            error!(target: LOG_TARGET, "📝 invalid bls signature: {:?}", e);
            Error::InvalidBlsSignature
        })?;

        let bls_valid =
            <Bls as CryptoScheme>::verify(&bls_pubkey, &attestation.signature_bls, &msg);

        Ok(sr_valid && bls_valid)
    }
}

impl<Block, AccountId, RA: ProvideRuntimeApi<Block>, BE> Validator<Block>
    for AttestorGossipValidator<Block, AccountId, RA, BE>
where
    Block: BlockT,
    H256: From<<Block as BlockT>::Hash>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
    RA: ProvideRuntimeApi<Block> + Send + Sync + 'static,
    RA::Api: AttestorApi<Block, HashFor<Block>, AccountId>,
    RA::Api: SupportedChainsApi<Block>,
    RA::Api: RandomnessPalletApi<Block>,
    BE: Backend<Block>,
    AccountId: Clone + Display + Codec + Send + 'static + Sync + Debug + Into<[u8; 32]>,
{
    fn validate(
        &self,
        context: &mut dyn ValidatorContext<Block>,
        sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Block::Hash> {
        let action = match Message::<Block, AccountId>::decode(&mut &data[..]) {
            Ok(Message::Attestation(att)) => {
                info!(target: LOG_TARGET, "📝 Received attestation by: {:?}", att.attestor);
                self.validate_attestation(&att, sender).unwrap_or_else(|err| {
                    error!(target: LOG_TARGET, "📝 Error decoding block hash in message: {:?}", err);
                    Action::Discard
                })
            }
            Err(err) => {
                error!(target: LOG_TARGET, "📝 Error decoding block hash in message: {:?}", err);
                Action::Discard
            }
        };

        match action {
            Action::Keep(topic) => {
                info!(target: LOG_TARGET, "📝 Broadcasting message for topic {:?}", topic);
                context.broadcast_message(topic, data.to_vec(), true);
                ValidationResult::ProcessAndKeep(topic)
            }
            Action::Discard => ValidationResult::Discard,
        }
    }
}
