use std::marker::PhantomData;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use attestor_primitives::{AttestationData, Felt};
use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    FutureExt as _, StreamExt,
};
use parity_scale_codec::{Decode, Encode};
use parking_lot::Mutex;
use sc_network::{PeerId, ProtocolName};
use sc_network_gossip::{GossipEngine, ValidationResult, Validator, ValidatorContext};
use serde::Serialize;
use sp_core::{Pair, H256, U256};
use sp_runtime::{
    traits::{Block as BlockT, Hash},
    AccountId32,
};
use substrate_prometheus_endpoint::Registry;

pub struct AttestorGossipValidator<B>
where
    B: BlockT,
{
    _phantom: PhantomData<B>,
}

impl<B> Default for AttestorGossipValidator<B>
where
    B: BlockT<Hash = H256>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<B> AttestorGossipValidator<B>
where
    B: BlockT<Hash = H256>,
{
    pub fn new() -> Self {
        Self {
            _phantom: Default::default(),
        }
    }

    fn validate_attestation(
        &self,
        attestation: &Attestation<B::Hash>,
        _sender: &PeerId,
    ) -> Action<B::Hash> {
        let round = attestation.round;
        let x = self.verify_signature(attestation);
        if x {
            log::info!(target: "attestor-gossip", "Attestation signature is valid");
            Action::Keep(round_topic::<B>(round, attestation.topic.clone()))
        } else {
            // TODO: slash stake in the future
            log::info!(target: "attestor-gossip", "Attestation signature is invalid");
            Action::Discard
        }
    }

    // Check it the signature is valid given the header number and header hash from the attestation for now.
    // Will need extending once we start submitting actual attestations
    fn verify_signature(&self, attestation: &Attestation<B::Hash>) -> bool {
        let header_number = attestation.header_number;
        let header_hash = attestation.header_hash;
        let tx_root = attestation.tx_root;
        let rx_root = attestation.rx_root;
        let att = AttestationData {
            header_number,
            header_hash,
            tx_root,
            rx_root,
        };

        let message = att.serialize();
        let signature = &attestation.signature;
        let public_key: sp_core::sr25519::Public =
            sp_core::sr25519::Public::from_raw(attestation.attestor.0.clone().into());

        sp_core::sr25519::Pair::verify(signature, message, &public_key)
    }

    fn validate_attestation_request(&self) -> Action<B::Hash> {
        Action::Discard
    }
}

#[derive(Debug, PartialEq)]
pub enum Action<H> {
    Keep(H),
    Discard,
}

pub type Round = u64;

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttestorId(AccountId32);

impl AttestorId {
    pub fn new(id: AccountId32) -> Self {
        Self(id)
    }

    pub fn from_public(public_key: [u8; 32]) -> Self {
        Self(AccountId32::new(public_key))
    }
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Topic(u64);

impl Topic {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

pub fn round_topic<B: BlockT>(round: Round, topic: Topic) -> B::Hash {
    let mut round_topic = Vec::new();
    round.encode_to(&mut round_topic);
    topic.encode_to(&mut round_topic);
    <<B as BlockT>::Header as sp_runtime::traits::Header>::Hashing::hash(&round_topic)
}

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attestation<H>
where
    H: Serialize,
{
    pub round: u64,
    pub header_hash: H,
    pub header_number: u64,
    pub tx_root: Felt,
    pub rx_root: Felt,
    pub attestor: AttestorId,
    pub topic: Topic,
    pub vrf_output: (U256, u32),
    pub signature: sp_core::sr25519::Signature,
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq)]
pub enum Message<H>
where
    H: Serialize,
{
    Attestation(Attestation<H>),
    AttestationRequest {
        round: u64,
        topic: Topic,
        attestor: AttestorId,
    },
}

impl<H: Serialize> Message<H> {
    pub fn round_topic<B: BlockT>(&self) -> H
    where
        H: From<<B as BlockT>::Hash> + Serialize,
    {
        let (round, topic) = match self {
            Message::Attestation(Attestation { round, topic, .. }) => (round, topic),
            Message::AttestationRequest { round, topic, .. } => (round, topic),
        };
        round_topic::<B>(*round, topic.clone()).into()
    }
}

impl<B> Validator<B> for AttestorGossipValidator<B>
where
    B: BlockT<Hash = H256>,
{
    fn validate(
        &self,
        context: &mut dyn ValidatorContext<B>,
        sender: &PeerId,
        data: &[u8],
    ) -> ValidationResult<B::Hash> {
        let action = match Message::<B::Hash>::decode(&mut &data[..]) {
            Ok(Message::Attestation(att)) => {
                log::info!(target: "attestor-gossip", "Received attestation: {:?}", att);
                self.validate_attestation(&att, sender)
            }
            Ok(Message::AttestationRequest {
                round,
                topic,
                attestor,
            }) => {
                log::info!(target: "attestor-gossip", "Received attestation request from {:?} for round {:?} and topic {:?}", attestor, round, topic);
                self.validate_attestation_request()
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

pub trait Network<B: BlockT>: sc_network_gossip::Network<B> + Clone + Send + 'static {}
impl<B: BlockT, N: sc_network_gossip::Network<B> + Clone + Send + 'static> Network<B> for N {}

pub trait Syncing<B: BlockT>: sc_network_gossip::Syncing<B> + Clone + Send + 'static {}
impl<B: BlockT, S: sc_network_gossip::Syncing<B> + Clone + Send + 'static> Syncing<B> for S {}

pub struct Networking<B: BlockT, N: Network<B>, S: Syncing<B>> {
    #[allow(dead_code)]
    network: N,
    #[allow(dead_code)]
    sync: S,
    gossip_engine: Arc<Mutex<GossipEngine<B>>>,
    #[allow(dead_code)]
    validator: Arc<AttestorGossipValidator<B>>,
    msg_stream: Mutex<UnboundedReceiver<Message<HashFor<B>>>>,
}

pub type HashFor<B> = <B as BlockT>::Hash;

pub struct BestKnown<H> {
    pub hash: H,
    pub number: u64,
}

pub struct State<H> {
    pub round: Round,
    pub attestor: AttestorId,
    pub best: Option<BestKnown<H>>,
}

impl<B: BlockT<Hash = H256>, N: Network<B>, S: Syncing<B>> Networking<B, N, S> {
    fn new(
        network: N,
        sync: S,
        protocol_name: ProtocolName,
        validator: Arc<AttestorGossipValidator<B>>,
        prometheus_registry: Option<&Registry>,
        msg_stream: UnboundedReceiver<Message<HashFor<B>>>,
    ) -> Self {
        let gossip_engine = Arc::new(Mutex::new(GossipEngine::new(
            network.clone(),
            sync.clone(),
            protocol_name,
            validator.clone(),
            prometheus_registry,
        )));
        Self {
            network,
            sync,
            gossip_engine,
            validator,
            msg_stream: Mutex::new(msg_stream),
        }
    }
}

impl<B: BlockT, N: Network<B>, S: Syncing<B>> Future for Networking<B, N, S> {
    type Output = Result<(), Error>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.msg_stream.lock().poll_next_unpin(cx) {
            Poll::Ready(Some(message)) => {
                log::info!(target: "attestor-gossip", "Got message to gossip {:?}", message);
                self.gossip_engine.lock().gossip_message(
                    message.round_topic::<B>(),
                    message.encode(),
                    false,
                );
            }
            Poll::Ready(None) => {}
            Poll::Pending => {}
        }

        match self.gossip_engine.lock().poll_unpin(cx) {
            Poll::Ready(()) => return Poll::Ready(Err(Error::GossipEngineExited)),
            Poll::Pending => {}
        }

        Poll::Pending
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Gossip engine exited")]
    GossipEngineExited,
}

fn networking<B, N, S>(
    network: N,
    sync: S,
    protocol_name: ProtocolName,
    prometheus_registry: Option<&Registry>,
    msg_stream: UnboundedReceiver<Message<HashFor<B>>>,
) -> Networking<B, N, S>
where
    B: BlockT<Hash = H256>,
    N: Network<B>,
    S: Syncing<B>,
{
    let validator = Arc::new(AttestorGossipValidator::new());
    Networking::new(
        network,
        sync,
        protocol_name,
        validator,
        prometheus_registry,
        msg_stream,
    )
}

pub type MessageSink<B> = UnboundedSender<Message<HashFor<B>>>;

pub fn start<B, N, S>(
    network: N,
    sync: S,
    protocol_name: ProtocolName,
    prometheus_registry: Option<&Registry>,
) -> (impl Future<Output = ()>, MessageSink<B>)
where
    B: BlockT<Hash = H256>,
    N: Network<B>,
    S: Syncing<B>,
{
    let (msg_sender, msg_stream) = futures::channel::mpsc::unbounded();
    (
        networking(
            network,
            sync,
            protocol_name,
            prometheus_registry,
            msg_stream,
        )
        .map(|res| {
            if let Err(err) = res {
                log::error!(target: "attestor-gossip", "Networking exited with error: {}", err);
            }
        }),
        msg_sender,
    )
}

pub fn peers_set_config(protocol_name: ProtocolName) -> sc_network::config::NonDefaultSetConfig {
    sc_network::config::NonDefaultSetConfig {
        notifications_protocol: protocol_name,
        fallback_names: vec![],
        // Notifications reach ~256kiB in size at the time of writing on Kusama and Polkadot.
        max_notification_size: 1024 * 1024,
        handshake: None,
        set_config: sc_network::config::SetConfig {
            in_peers: 0,
            out_peers: 0,
            reserved_nodes: Vec::new(),
            non_reserved_mode: sc_network::config::NonReservedPeerMode::Deny,
        },
    }
}
