use parity_scale_codec::Codec;
use sc_client_api::{client::BlockBackend, Backend};
use sc_network::ProtocolName;
use sc_network_gossip::{GossipEngine, Network as GossipNetwork, Syncing as GossipSyncing};
use sc_utils::mpsc::{TracingUnboundedReceiver, TracingUnboundedSender};

use sp_api::ProvideRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
use std::fmt::{Debug, Display};
use std::marker::PhantomData;
use std::sync::Arc;
use substrate_prometheus_endpoint::Registry;
use thiserror::Error;

use attestor_primitives::{api::AttestorApi, AttestorId};
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;
use worker::{Worker, WorkerParams};

pub mod communication;
pub mod inherent;
mod round;
mod state;
mod validator;
mod worker;

use communication::gossip::Message;
use validator::AttestorGossipValidator;

pub type HashFor<B> = <B as BlockT>::Hash;

pub type MessageSink<B, A> = TracingUnboundedSender<Message<B, A>>;

const LOG_TARGET: &str = "attestor-gossip";

pub(crate) struct AttestorComms<B: BlockT, AccountId, RA: ProvideRuntimeApi<B>, BE>
where
    RA: ProvideRuntimeApi<B> + Send + Sync + 'static,
    RA::Api: AttestorApi<B, HashFor<B>, AccountId>,
    BE: Backend<B> + 'static,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
{
    pub gossip_engine: GossipEngine<B>,
    #[allow(dead_code)]
    pub gossip_validator: Arc<AttestorGossipValidator<B, AccountId, RA, BE>>,
    pub gossip_report_stream: TracingUnboundedReceiver<Message<B, AccountId>>,
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
    #[error("Attestation to old")]
    AttestationTooOld,
    #[error("Attestation to early")]
    AttestationTooEarly,
    #[error("Attestation header number invalid")]
    AttestationHeaderNumberInvalid,
    #[error("Error creating inherent data")]
    ErrorCreatingInherent,
    #[error("Sender {0:?} is not an attestor")]
    NotAnAttestor(AttestorId),
    #[error("Digest missmatch")]
    DigestMissMatch,
    #[error("Failed to fetch last digest")]
    FetchLastDigestError,
    #[error("Invalid bls signature")]
    InvalidBlsSignature,
    #[error("Invalid sr signature")]
    InvalidSrSignature,
    #[error("Chain not supported")]
    ChainNotSupported,
    #[error("Sp api error")]
    SpApiError(#[from] sp_api::ApiError),
    #[error("Vrf error")]
    VrfError(#[from] vrf::Error),
    #[error("Attestor {0:?} not eligible")]
    AttestorNotEligible(AttestorId),
    #[error("Attestor {0:?} not active")]
    AttestorNotActive(AttestorId),
    #[error("Failed to get attestation interval")]
    FailedToGetAttestationInterval,
    #[error("Failed to get last attestation after existance confirmed")]
    FailedToGetLastAttestation,
    #[error("Failed to get round configuration")]
    RoundConfigNotSet,
    #[error("Other error: {0}")]
    Other(String),
    #[error("Finality stream terminated")]
    FinalityStreamTerminated,
    #[error("Attestation data contains invalid epoch")]
    InvalidEpoch,
    #[error("Attestation contains an invalid continuity proof")]
    InvalidAttestationContinuityProof,
    #[error("Attestation contains an invalid epoch mismatch")]
    EpochMismatch,
}

/// Attestor gadget network parameters.
pub struct AttestorNetworkParams<B: BlockT, N, S, AccountId> {
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
    pub msg_stream: TracingUnboundedReceiver<Message<B, AccountId>>,
    pub phantom: PhantomData<B>,
}

pub struct AttestorGossipParams<B: BlockT, BE, C, N, R, S, CIDP, AccountId> {
    /// Attestor client
    pub client: Arc<C>,
    /// Client Backend
    pub backend: Arc<BE>,
    /// Runtime Api Provider
    pub runtime: Arc<R>,
    /// Attestor voter network params
    pub network_params: AttestorNetworkParams<B, N, S, AccountId>,
    /// Minimal delta between blocks, BEEFY should vote for
    pub min_block_delta: u32,
    /// Prometheus metric registry
    pub prometheus_registry: Option<Registry>,
    /// Inherent data providers
    pub create_inherent_data_providers: CIDP,
    pub inherent_provider: inherent::AsyncProvider<AccountId, B, R, BE>,
    pub _phantom: PhantomData<AccountId>,
}

pub async fn start_attestor_gossip_gadget<B, BE, C, N, R, S, CIDP, AccountId>(
    attestor_params: AttestorGossipParams<B, BE, C, N, R, S, CIDP, AccountId>,
) where
    B: BlockT,
    BE: Backend<B> + 'static,
    C: Client<B, BE> + BlockBackend<B>,
    R: ProvideRuntimeApi<B> + Send + Sync + 'static,
    R::Api: BabeApi<B>,
    R::Api: AttestorApi<B, HashFor<B>, AccountId>,
    R::Api: SupportedChainsApi<B>,
    R::Api: RandomnessPalletApi<B>,
    N: GossipNetwork<B> + Send + Sync + 'static,
    S: GossipSyncing<B> + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
    CIDP: CreateInherentDataProviders<B, ()> + 'static,
    <<B as BlockT>::Header as HeaderT>::Number: Into<u64>,
    AccountId: Clone
        + Display
        + Codec
        + Send
        + 'static
        + Sync
        + Debug
        + Into<[u8; 32]>
        + PartialEq
        + Eq
        + std::hash::Hash,
{
    let AttestorGossipParams {
        client,
        runtime,
        network_params,
        create_inherent_data_providers,
        backend,
        inherent_provider,
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

    // Subscribe to finality notifications and justifications before waiting for runtime pallet and
    // reuse the streams, so we don't miss notifications while waiting for pallet to be available.
    let finality_notifications = client.finality_notification_stream();

    let gossip_validator =
        AttestorGossipValidator::<B, AccountId, R, BE>::new(runtime.clone(), backend.clone());
    let validator: Arc<AttestorGossipValidator<B, AccountId, R, BE>> = Arc::new(gossip_validator);
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
        backend: backend.clone(),
        inherent_provider,
        _phantom: PhantomData,
    };

    let worker: Worker<B, R, BE, C, CIDP, AccountId> = Worker::new(worker_params);

    worker.start(finality_notifications).await;
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
