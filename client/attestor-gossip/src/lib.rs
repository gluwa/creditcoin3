use futures::{stream::Fuse, FutureExt, StreamExt};
use log::{debug, error, info};
use parity_scale_codec::Codec;
use sc_client_api::{client::BlockBackend, Backend, FinalityNotification};
use sc_network::{NotificationService, ProtocolName};
use sc_network_gossip::{GossipEngine, Network as GossipNetwork, Syncing as GossipSyncing};
use sc_utils::mpsc::{tracing_unbounded, TracingUnboundedReceiver, TracingUnboundedSender};
use sp_api::ProvideRuntimeApi;
use sp_consensus::SyncOracle;
use sp_consensus_babe::BabeApi;
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
use std::{
    fmt::{Debug, Display},
    future::Future,
    pin::Pin,
    sync::Arc,
};
mod metrics;
use substrate_prometheus_endpoint::Registry;

use attestor_primitives::api::AttestorApi;
use randomness_primitives::api::RandomnessPalletApi;
use supported_chains_primitives::api::SupportedChainsApi;
use worker::{Worker, WorkerParams};

pub mod communication;
pub mod inherent;
mod round;
mod state;
mod validate;
mod worker;

#[cfg(test)]
mod test_utils;

use communication::{gossip::Message, validator::AttestorGossipValidator, Error};

pub type HashFor<B> = <B as BlockT>::Hash;

pub type MessageSink<B, A> = TracingUnboundedSender<Message<B, A>>;

const LOG_TARGET: &str = "attestor-gossip";

type FinalityNotifications<Block> =
    sc_utils::mpsc::TracingUnboundedReceiver<UnpinnedFinalityNotification<Block>>;

pub(crate) struct AttestorComms<B: BlockT, AccountId>
where
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
    pub gossip_validator: Arc<AttestorGossipValidator<B, AccountId>>,
    pub gossip_report_stream: TracingUnboundedReceiver<Message<B, AccountId>>,
    // pub on_demand_justifications: OnDemandJustificationsEngine<B>,
}

/// Attestor gadget network parameters.
pub struct AttestorNetworkParams<B: BlockT, N, S, AccountId> {
    /// Network implementing gossip, requests and sync-oracle.
    pub network: Arc<N>,
    /// Syncing service implementing a sync oracle and an event stream for peers.
    pub sync: Arc<S>,
    /// Handle for receiving notification events.
    pub notification_service: Box<dyn NotificationService>,
    /// Chain specific Attestor gossip protocol name. See
    /// [`communication::attestor_protocol_name::gossip_protocol_name`].
    pub gossip_protocol_name: ProtocolName,
    /// External rpc message stream
    pub msg_stream: TracingUnboundedReceiver<Message<B, AccountId>>,
}

pub struct AttestorGossipParams<B: BlockT, BE, C, N, R, S, AccountId> {
    /// Attestor client
    pub client: Arc<C>,
    /// Client Backend
    pub backend: Arc<BE>,
    /// Runtime Api Provider
    pub runtime: Arc<R>,
    /// Attestor voter network params
    pub network_params: AttestorNetworkParams<B, N, S, AccountId>,
    /// Prometheus metric registry
    pub prometheus_registry: Option<Registry>,
    /// Handler to provide inherent data to the consensus engine.
    pub inherent_provider: inherent::AsyncProvider<AccountId, B, R, BE>,
    /// Is the node is an authority
    pub is_authority: bool,
}

/// Finality notification for consumption by BEEFY worker.
/// This is a stripped down version of `sc_client_api::FinalityNotification` which does not keep
/// blocks pinned.
struct UnpinnedFinalityNotification<B: BlockT> {
    /// Finalized block header hash.
    pub hash: B::Hash,
    /// Finalized block header.
    pub header: B::Header,
    /// Path from the old finalized to new finalized parent (implicitly finalized blocks).
    ///
    /// This maps to the range `(old_finalized, new_finalized)`.
    pub tree_route: Arc<[B::Hash]>,
}

impl<B: BlockT> From<FinalityNotification<B>> for UnpinnedFinalityNotification<B> {
    fn from(value: FinalityNotification<B>) -> Self {
        UnpinnedFinalityNotification {
            hash: value.hash,
            header: value.header,
            tree_route: value.tree_route,
        }
    }
}

/// Produce a future that transformes finality notifications into a struct that does not keep blocks
/// pinned.
fn finality_notification_transformer_future<B>(
    mut finality_notifications: sc_client_api::FinalityNotifications<B>,
) -> (
    Pin<Box<futures::future::Fuse<impl Future<Output = ()> + Sized>>>,
    Fuse<TracingUnboundedReceiver<UnpinnedFinalityNotification<B>>>,
)
where
    B: BlockT,
{
    let (tx, rx) = tracing_unbounded("attestor-notification-transformer-channel", 10000);
    debug!(target: LOG_TARGET, "📝 Starting finality notification transformer.");
    let transformer_fut = async move {
        while let Some(notification) = finality_notifications.next().await {
            info!(target: LOG_TARGET, "📝 Transforming grandpa notification. #{}({:?})", notification.header.number(), notification.hash);
            if let Err(err) = tx.unbounded_send(UnpinnedFinalityNotification::from(notification)) {
                error!(target: LOG_TARGET, "📝 Unable to send transformed notification. Shutting down. err = {}", err);
                return;
            };
        }
    };
    (Box::pin(transformer_fut.fuse()), rx.fuse())
}

pub async fn start_attestor_gossip_gadget<B, BE, C, N, R, S, AccountId>(
    attestor_params: AttestorGossipParams<B, BE, C, N, R, S, AccountId>,
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
    S: GossipSyncing<B> + SyncOracle + 'static,
    H256: From<<B as BlockT>::Hash>,
    <B as BlockT>::Hash: From<H256>,
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
        backend,
        inherent_provider,
        is_authority,
        prometheus_registry,
        ..
    } = attestor_params;

    let AttestorNetworkParams {
        network,
        sync,
        notification_service,
        gossip_protocol_name,
        // justifications_protocol_name,
        msg_stream,
        ..
    } = network_params;

    // Subscribe to finality notifications and justifications before waiting for runtime pallet and
    // reuse the streams, so we don't miss notifications while waiting for pallet to be available.
    let finality_notifications = client.finality_notification_stream();
    let (mut transformer, mut finality_notifications) =
        finality_notification_transformer_future(finality_notifications);

    let gossip_validator = AttestorGossipValidator::<B, AccountId>::default();
    let gossip_validator = Arc::new(gossip_validator);
    let gossip_engine = GossipEngine::new(
        network.clone(),
        sync.clone(),
        notification_service,
        gossip_protocol_name.clone(),
        gossip_validator.clone(),
        None,
    );
    let mut attestor_comms = AttestorComms {
        gossip_engine,
        gossip_validator,
        gossip_report_stream: msg_stream,
    };

    // We re-create and re-run the worker in this loop in order to quickly reinit and resume after
    // select recoverable errors.
    loop {
        let worker_params = WorkerParams {
            comms: attestor_comms,
            runtime: runtime.clone(),
            client: client.clone(),
            backend: backend.clone(),
            inherent_provider: inherent_provider.clone(),
            is_authority,
            sync: sync.clone(),
            prometheus_registry: prometheus_registry.clone(),
        };
        let worker: Worker<B, R, BE, C, AccountId, S> = Worker::new(worker_params);

        futures::select! {
            result = worker.start(&mut finality_notifications).fuse() => {
                match result {
                    (Error::GossipEngineExited, reuse_comms) => {
                        error!(target: LOG_TARGET, "📝 Gossip engine has exited.");
                        attestor_comms = reuse_comms;
                        continue;
                    },
                    _ => {
                        error!(target: LOG_TARGET, "📝 Attestor worker has exited.");
                    }
                }

            },
            _ = &mut transformer => {
                error!(target: LOG_TARGET, "📝 Finality notification transformer has exited.");
            },
        }
        return;
    }
}

pub fn peers_set_config<
    B: sp_runtime::traits::Block,
    N: sc_network::NetworkBackend<B, <B as sp_runtime::traits::Block>::Hash>,
>(
    gossip_protocol_name: sc_network::ProtocolName,
    metrics: sc_network::service::NotificationMetrics,
    peer_store_handle: std::sync::Arc<dyn sc_network::peer_store::PeerStoreProvider>,
) -> (
    N::NotificationProtocolConfig,
    Box<dyn sc_network::NotificationService>,
) {
    let (cfg, notification_service) = N::notification_config(
        gossip_protocol_name,
        Vec::new(),
        1024 * 1024,
        None,
        sc_network::config::SetConfig {
            in_peers: 25,
            out_peers: 25,
            reserved_nodes: Vec::new(),
            non_reserved_mode: sc_network::config::NonReservedPeerMode::Accept,
        },
        metrics,
        peer_store_handle,
    );
    (cfg, notification_service)
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
