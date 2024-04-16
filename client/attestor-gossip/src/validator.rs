use attestor_primitives::AttestationData;
use parity_scale_codec::Decode;
use parity_scale_codec::Encode;
use sc_network::PeerId;
use sc_network_gossip::{ValidationResult, Validator, ValidatorContext};
use sc_utils::mpsc::TracingUnboundedReceiver;
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedSender};
use serde::Serialize;
use sp_api::HeaderT;
use sp_core::{Pair, H256};
use sp_runtime::traits::{Block as BlockT, Hash};
use std::marker::PhantomData;

use crate::HashFor;

use super::{Action, Attestation, Error, Message};

pub struct AttestorGossipValidator<Block>
where
    Block: BlockT,
{
    _phantom: PhantomData<Block>,
    report_sender: TracingUnboundedSender<Message<HashFor<Block>>>,
}

impl<B> AttestorGossipValidator<B>
where
    B: BlockT,
    H256: From<<B as BlockT>::Hash>,
{
    pub fn new() -> (Self, TracingUnboundedReceiver<Message<HashFor<B>>>) {
        let (tx, rx) = tracing_unbounded("mpsc_attestor_gossip_validator", 100_000);

        (
            Self {
                _phantom: Default::default(),
                report_sender: tx,
            },
            rx,
        )
    }

    pub fn validate_attestation(
        &self,
        attestation: &Attestation<HashFor<B>>,
        _sender: &PeerId,
    ) -> Result<Action<B::Hash>, Error> {
        let round = attestation.round;
        let valid_sig = self.verify_signature(attestation);
        if !valid_sig {
            log::info!(target: "attestor-gossip", "Attestation signature is invalid");
            return Err(Error::InvalidAttestationDataSignature);
        };

        log::info!(target: "attestor-gossip", "Attestation signature is valid");
        Ok(Action::Keep(round_topic::<B>(
            round,
            attestation.topic.clone(),
        )))
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
    <<Block as BlockT>::Header as HeaderT>::Number: From<sp_core::U256>,
    H256: From<<Block as BlockT>::Hash>,
{
    fn validate(
        &self,
        context: &mut dyn ValidatorContext<Block>,
        sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<Block::Hash> {
        let action = match Message::<Block::Hash>::decode(&mut &data[..]) {
            Ok(Message::Attestation(att)) => {
                log::info!(target: "attestor-gossip", "Received attestation: {:?}", att);
                match self.validate_attestation(&att, sender) {
                    Ok(a) => a,
                    Err(err) => {
                        log::error!(target: "attestor-gossip", "Error decoding block hash in message: {:?}", err);
                        Action::Discard
                    }
                }
            }
            Err(err) => {
                log::error!(target: "attestor-gossip", "Error decoding block hash in message: {:?}", err);
                Action::Discard
            }
        };

        match action {
            Action::Keep(topic) => {
                log::info!(target: "attestor-gossip", "Broadcasting message for topic {:?}", topic);
                context.broadcast_message(topic, data.to_vec(), false);
                ValidationResult::ProcessAndKeep(topic)
            }
            Action::Discard => ValidationResult::Discard,
        }
    }
}

// TODO, what is this
use super::{Round, Topic};

impl<H: Serialize> Message<H> {
    pub fn round_topic<B: BlockT>(&self) -> H
    where
        H: From<<B as BlockT>::Hash> + Serialize,
    {
        let (round, topic) = match self {
            Message::Attestation(Attestation { round, topic, .. }) => (round, topic),
        };
        round_topic::<B>(*round, topic.clone()).into()
    }
}

pub fn round_topic<B: BlockT>(round: Round, topic: Topic) -> B::Hash {
    let mut round_topic = Vec::new();
    round.encode_to(&mut round_topic);
    topic.encode_to(&mut round_topic);
    <<B as BlockT>::Header as sp_runtime::traits::Header>::Hashing::hash(&round_topic)
}
