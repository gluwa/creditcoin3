use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    channel::mpsc::{UnboundedReceiver, UnboundedSender},
    FutureExt as _, StreamExt,
};
use parity_scale_codec::{Decode, Encode};
use parking_lot::Mutex;
use sc_network::ProtocolName;
use sc_network_gossip::{GossipEngine, Validator};
use serde::Serialize;
use sp_runtime::{
    traits::{Block as BlockT, Hash},
    AccountId32,
};
use substrate_prometheus_endpoint::Registry;

pub struct AttestorGossipValidator {}

impl AttestorGossipValidator {
    pub const fn new() -> Self {
        Self {}
    }
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

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attestation<H>
where
    H: Serialize,
{
    pub round: u64,
    pub header_hash: H,
    pub header_number: u64,
    pub attestor: AttestorId,
    pub topic: Topic,
    pub vrf_output: u64,
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

impl<B: BlockT> Validator<B> for AttestorGossipValidator {
    fn validate(
        &self,
        _context: &mut dyn sc_network_gossip::ValidatorContext<B>,
        _sender: &sc_network::PeerId,
        data: &[u8],
    ) -> sc_network_gossip::ValidationResult<<B as BlockT>::Hash> {
        let message = match Message::<B::Hash>::decode(&mut &data[..]) {
            Ok(message) => message,
            Err(err) => {
                log::error!(target: "attestor-gossip", "Error decoding block hash in message: {:?}", err);
                return sc_network_gossip::ValidationResult::Discard;
            }
        };

        log::info!(target: "attestor-gossip", "Received message: {:?}", message);
        let topic = message.round_topic::<B>();

        sc_network_gossip::ValidationResult::ProcessAndKeep(topic)
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
    validator: Arc<AttestorGossipValidator>,
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

impl<B: BlockT, N: Network<B>, S: Syncing<B>> Networking<B, N, S> {
    fn new(
        network: N,
        sync: S,
        protocol_name: ProtocolName,
        validator: Arc<AttestorGossipValidator>,
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
    B: BlockT,
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
    B: BlockT,
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
