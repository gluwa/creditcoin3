use attestor_primitives::AttestationData;
use parity_scale_codec::Decode;
use parity_scale_codec::Encode;
use sc_network::PeerId;
use sc_network_gossip::{ValidationResult, Validator, ValidatorContext};
use sp_core::{Pair, H256};
use sp_runtime::traits::{Block as BlockT, Hash};
use std::marker::PhantomData;

use crate::{worker::votes_topic, HashFor, LOG_TARGET};

use super::{Action, Attestation, Error, Message};

pub struct AttestorGossipValidator<Block>
where
    Block: BlockT,
{
    _phantom: PhantomData<Block>,
}

impl<B> AttestorGossipValidator<B>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
{
    pub fn new() -> Self {
        Self {
            _phantom: Default::default(),
        }
    }

    pub fn validate_attestation(
        &self,
        attestation: &Attestation<HashFor<B>>,
        _sender: &PeerId,
    ) -> Result<Action<B::Hash>, Error> {
        let valid_sig = self.verify_signature(attestation);
        if !valid_sig {
            log::info!(target: LOG_TARGET, "📝 Attestation signature is invalid");
            return Err(Error::InvalidAttestationDataSignature);
        };

        log::info!(target: LOG_TARGET, "📝 Attestation signature is valid");
        Ok(Action::Keep(votes_topic::<B>()))
    }

    // Check it the signature is valid given the header number and header hash from the attestation for now.
    // Will need extending once we start submitting actual attestations
    fn verify_signature(&self, attestation: &Attestation<HashFor<B>>) -> bool {
        let h = H256::from(attestation.header_hash);

        let msg = AttestationData {
            header_number: attestation.header_number,
            header_hash: h,
            tx_root: attestation.tx_root,
            rx_root: attestation.rx_root,
        };

        let public_key = sp_core::sr25519::Public::from_raw(attestation.attestor.0.clone().into());

        sp_core::sr25519::Pair::verify(&attestation.signature, msg.serialize(), &public_key)
    }
}

impl<Block> Validator<Block> for AttestorGossipValidator<Block>
where
    Block: BlockT,
    H256: From<<Block as BlockT>::Hash>,
{
    fn validate(
        &self,
        context: &mut dyn ValidatorContext<Block>,
        sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Block::Hash> {
        let action = match Message::<Block>::decode(&mut &data[..]) {
            Ok(Message::Attestation(att)) => {
                log::info!(target: LOG_TARGET, "📝 Received attestation: {:?}", att);
                match self.validate_attestation(&att, sender) {
                    Ok(a) => a,
                    Err(err) => {
                        log::error!(target: LOG_TARGET, "📝 Error decoding block hash in message: {:?}", err);
                        Action::Discard
                    }
                }
            }
            Err(err) => {
                log::error!(target: LOG_TARGET, "📝 Error decoding block hash in message: {:?}", err);
                Action::Discard
            }
        };

        match action {
            Action::Keep(topic) => {
                log::info!(target: LOG_TARGET, "📝 Broadcasting message for topic {:?}", topic);
                context.broadcast_message(topic, data.to_vec(), false);
                ValidationResult::ProcessAndKeep(topic)
            }
            Action::Discard => ValidationResult::Discard,
        }
    }
}

// TODO, what is this
use super::{Round, Topic};

impl<B> Message<B>
where
    B: BlockT,
{
    pub fn round_topic<Block: BlockT>(&self) -> B::Hash {
        match self {
            Message::Attestation(attestation) => {
                let (round, topic) = (attestation.round, attestation.topic.clone());
                round_topic::<B>(round, topic).into()
            }
        }
    }
}

pub fn round_topic<B: BlockT>(round: Round, topic: Topic) -> B::Hash {
    let mut round_topic = Vec::new();
    round.encode_to(&mut round_topic);
    topic.encode_to(&mut round_topic);
    <<B as BlockT>::Header as sp_runtime::traits::Header>::Hashing::hash(&round_topic)
}
