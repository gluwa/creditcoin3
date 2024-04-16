use sc_client_api::{client::BlockBackend, Backend};
use std::fmt::Debug;
use std::{marker::PhantomData, sync::Arc};
use thiserror::Error;
use worker::Worker;

use attestor_primitives::Felt;
use parity_scale_codec::{Decode, Encode};
use parking_lot::Mutex;
use sc_network::ProtocolName;
use sc_network_gossip::GossipEngine;
use sc_utils::mpsc::{TracingUnboundedReceiver, TracingUnboundedSender};
use serde::{Deserialize, Serialize};
use sp_api::{HeaderT, ProvideRuntimeApi};
use sp_consensus_babe::BabeApi;
use sp_core::{H256, U256};
use sp_runtime::{traits::Block as BlockT, AccountId32};
use substrate_prometheus_endpoint::Registry;

pub mod validator;
pub mod worker;

use validator::AttestorGossipValidator;

pub type HashFor<B> = <B as BlockT>::Hash;

pub type MessageSink<B> = TracingUnboundedSender<Message<HashFor<B>>>;

pub(crate) struct AttestorComms<B: BlockT> {
    pub gossip_engine: GossipEngine<B>,
    pub gossip_validator: Arc<AttestorGossipValidator<B>>,
    pub gossip_report_stream: TracingUnboundedReceiver<Message<HashFor<B>>>,
    // pub on_demand_justifications: OnDemandJustificationsEngine<B>,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Gossip engine exited")]
    GossipEngineExited,
    #[error("Invalid attestation signature")]
    InvalidAttestationDataSignature,
    #[error("Invalid attestation vrf output")]
    InvalidAttestationVrfOuput,
    #[error("Sp api error")]
    SpApiError(#[from] sp_api::ApiError),
}

#[derive(Debug, PartialEq)]
pub enum Action<H> {
    Keep(H),
    Discard,
}

pub type Round = u64;

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestorId(AccountId32);

impl AttestorId {
    pub fn new(id: AccountId32) -> Self {
        Self(id)
    }

    pub fn from_public(public_key: [u8; 32]) -> Self {
        Self(AccountId32::new(public_key))
    }
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic(u64);

impl Topic {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub vrf_output: VrfOutput,
    pub signature: sp_core::sr25519::Signature,
}

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VrfOutput {
    pub vrf_number: U256,
    pub block_hash: H256,
}

#[derive(Encode, Decode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message<H>
where
    H: Serialize,
{
    Attestation(Attestation<H>),
    // Could be more messages to drop / slash or ignore certain attestors
    // ...
}

pub trait Network<B: BlockT>: sc_network_gossip::Network<B> + Clone + Send + 'static {}
impl<B: BlockT, N: sc_network_gossip::Network<B> + Clone + Send + 'static> Network<B> for N {}

pub trait Syncing<B: BlockT>: sc_network_gossip::Syncing<B> + Clone + Send + 'static {}
impl<B: BlockT, S: sc_network_gossip::Syncing<B> + Clone + Send + 'static> Syncing<B> for S {}

pub struct BestKnown<H> {
    pub hash: H,
    pub number: u64,
}

pub struct State<H> {
    pub round: Round,
    pub attestor: AttestorId,
    pub best: Option<BestKnown<H>>,
}

/// Attestor gadget network parameters.
pub struct AttestorNetworkParams<B: BlockT, N, S> {
    /// Network implementing gossip, requests and sync-oracle.
    pub network: Arc<N>,
    /// Syncing service implementing a sync oracle and an event stream for peers.
    pub sync: Arc<S>,
    /// Handle for receiving notification events.
    // pub notification_service: Box<dyn NotificationService>,
    /// Chain specific Attestor gossip protocol name. See
    /// [`communication::attestor_protocol_name::gossip_protocol_name`].
    pub gossip_protocol_name: ProtocolName,
    /// Chain specific Attestor on-demand justifications protocol name. See
    /// [`communication::attestor_protocol_name::justifications_protocol_name`].
    pub justifications_protocol_name: ProtocolName,

    pub _phantom: PhantomData<B>,
}

pub struct AttestorGossipParams<B: BlockT, BE, N, R, S> {
    /// Attestor client
    // pub client: Arc<C>,
    /// Client Backend
    pub backend: Arc<BE>,
    /// Runtime Api Provider
    pub runtime: Arc<R>,
    /// Attestor voter network params
    pub network_params: AttestorNetworkParams<B, N, S>,
    /// Minimal delta between blocks, BEEFY should vote for
    pub min_block_delta: u32,
    /// Prometheus metric registry
    pub prometheus_registry: Option<Registry>,
}

pub fn start_attestor_gossip_gadget<B, BE, N, R, S>(
    attestor_params: AttestorGossipParams<B, BE, N, R, S>,
) where
    B: BlockT,
    BE: Backend<B>,
    R: ProvideRuntimeApi<B> + Send + Sync + 'static,
    R::Api: BabeApi<B>,
    R::Api: BlockBackend<B>,
    N: Network<B> + Send + Sync,
    S: Syncing<B>,
    <<B as BlockT>::Header as HeaderT>::Number: From<sp_core::U256>,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
{
    let AttestorGossipParams {
        // client,
        backend,
        runtime,
        network_params,
        min_block_delta,
        prometheus_registry,
    } = attestor_params;

    let AttestorNetworkParams {
        network,
        sync,
        // notification_service,
        gossip_protocol_name,
        justifications_protocol_name,
        ..
    } = network_params;

    let (gossip_validator, gossip_report_stream) = AttestorGossipValidator::<B>::new();
    let validator: Arc<AttestorGossipValidator<B>> = Arc::new(gossip_validator);
    let gossip_engine = GossipEngine::new(
        network.clone(),
        sync.clone(),
        gossip_protocol_name.clone(),
        validator.clone(),
        None,
    );

    let mut attestor_comms = AttestorComms {
        gossip_engine,
        gossip_validator: validator,
        gossip_report_stream,
    };

    let worker: Worker<B, R> = Worker::new(attestor_comms, runtime.clone().into());

    worker.start();
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
