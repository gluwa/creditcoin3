use attestor_primitives::Felt;
use parity_scale_codec::{Decode, Encode};
use sc_client_api::{client::BlockBackend, Backend};
use sc_network::ProtocolName;
use sc_network_gossip::{GossipEngine, Network as GossipNetwork, Syncing as GossipSyncing};
use sc_utils::mpsc::{TracingUnboundedReceiver, TracingUnboundedSender};
use serde::{Deserialize, Serialize};
use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::{H256, U256};
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::{traits::Block as BlockT, AccountId32};
use std::fmt::Debug;
use std::{marker::PhantomData, sync::Arc};
use substrate_prometheus_endpoint::Registry;
use thiserror::Error;
use worker::{Worker, WorkerParams};

pub mod inherent;
pub mod validator;
pub mod worker;

use validator::AttestorGossipValidator;

use inherent::AttestationInherent;

pub type HashFor<B> = <B as BlockT>::Hash;

pub type MessageSink<B> = TracingUnboundedSender<Message<B>>;

const LOG_TARGET: &str = "attestor-gossip";

pub(crate) struct AttestorComms<B: BlockT> {
    pub gossip_engine: GossipEngine<B>,
    #[allow(dead_code)]
    pub gossip_validator: Arc<AttestorGossipValidator<B>>,
    pub gossip_report_stream: TracingUnboundedReceiver<Message<B>>,
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
    #[error("Error creating inherent data")]
    ErrorCreatingInherent,
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
pub struct Attestation<B> {
    pub round: u64,
    pub header_hash: B,
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
pub enum Message<B: BlockT> {
    Attestation(Attestation<HashFor<B>>),
    // Could be more messages to drop / slash or ignore certain attestors
    // ...
}

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
    /// External rpc message stream
    pub msg_stream: TracingUnboundedReceiver<Message<B>>,
    pub phantom: PhantomData<B>,
}

pub struct AttestorGossipParams<B: BlockT, BE, C, N, R, S, CIDP> {
    /// Attestor client
    pub client: Arc<C>,
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
    /// Inherent data providers
    pub create_inherent_data_providers: CIDP,
}

pub async fn start_attestor_gossip_gadget<B, BE, C, N, R, S, CIDP>(
    attestor_params: AttestorGossipParams<B, BE, C, N, R, S, CIDP>,
) where
    B: BlockT,
    BE: Backend<B>,
    C: Client<B, BE> + BlockBackend<B>,
    R: ProvideRuntimeApi<B> + Send + Sync + 'static,
    R::Api: BabeApi<B>,
    N: GossipNetwork<B> + Send + Sync + 'static,
    S: GossipSyncing<B> + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    CIDP: CreateInherentDataProviders<B, AttestationInherent<HashFor<B>>> + 'static,
{
    let AttestorGossipParams {
        client,
        runtime,
        network_params,
        create_inherent_data_providers,
        ..
    } = attestor_params;

    let AttestorNetworkParams {
        network,
        sync,
        // notification_service,
        gossip_protocol_name,
        // justifications_protocol_name,
        msg_stream,
        ..
    } = network_params;

    let gossip_validator = AttestorGossipValidator::<B>::new();
    let validator: Arc<AttestorGossipValidator<B>> = Arc::new(gossip_validator);
    let gossip_engine = GossipEngine::new(
        network.clone(),
        sync.clone(),
        gossip_protocol_name.clone(),
        validator.clone(),
        None,
    );

    let attestor_comms = AttestorComms {
        gossip_engine,
        gossip_validator: validator,
        gossip_report_stream: msg_stream,
    };

    let worker_params = WorkerParams {
        comms: attestor_comms,
        runtime: runtime.clone(),
        client: client.clone(),
        create_inherent_data_providers,
        backend: std::marker::PhantomData,
    };

    let worker: Worker<B, R, BE, C, CIDP> = Worker::new(worker_params);

    worker.start().await;
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

use sc_client_api::{BlockchainEvents, Finalizer, HeaderBackend};
/// A convenience Attestor client trait that defines all the type bounds a Attestor client
/// has to satisfy. Ideally that should actually be a trait alias. Unfortunately as
/// of today, Rust does not allow a type alias to be used as a trait bound. Tracking
/// issue is <https://github.com/rust-lang/rust/issues/41517>.
pub trait Client<B, BE>:
    BlockchainEvents<B> + HeaderBackend<B> + Finalizer<B, BE> + Send + Sync
where
    B: BlockT,
    BE: Backend<B>,
{
    // empty
}

impl<B, BE, T> Client<B, BE> for T
where
    B: BlockT,
    BE: Backend<B>,
    T: BlockchainEvents<B>
        + HeaderBackend<B>
        + Finalizer<B, BE>
        + ProvideRuntimeApi<B>
        + Send
        + Sync,
{
    // empty
}
