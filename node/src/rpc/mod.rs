//! A collection of node-specific RPC methods.

pub mod tracing;

use fp_rpc::EthereumRuntimeRPCApi;
use std::sync::Arc;

use futures::channel::mpsc;
use jsonrpsee::RpcModule;
// Substrate
use sc_client_api::{
    backend::{Backend, StorageProvider},
    client::BlockchainEvents,
    AuxStore, StateBackend, UsageProvider,
};
use sc_consensus_babe::BabeWorkerHandle;
use sc_consensus_grandpa::FinalityProofProvider;
use sc_consensus_manual_seal::rpc::EngineCommand;
use sc_rpc::SubscriptionTaskExecutor;
use sc_rpc_api::DenyUnsafe;
use sc_service::TaskManager;
use sc_service::TransactionPool;
use sc_transaction_pool::ChainApi;
use sp_api::{CallApiAt, ProvideRuntimeApi};
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
// Runtime
use creditcoin3_cli_opt::EthApi as EthApiCmd;
use creditcoin3_runtime::{opaque::Block, AccountId, Balance, BlockNumber, Hash, Nonce};
use fc_rpc::OverrideHandle;
use fc_rpc_core::types::{FeeHistoryCache, FilterPool};
use sc_client_api::BlockOf;
use sp_block_builder::BlockBuilder;
use sp_core::H256;
use sp_runtime::traits::BlakeTwo256;
use std::time::Duration;

mod eth;

use crate::client::RuntimeApiCollection;

pub use self::eth::{
    consensus_data_provider::BabeConsensusDataProvider, create_eth, overrides_handle, EthDeps,
};

type HasherFor<Block> = <<Block as BlockT>::Header as HeaderT>::Hashing;

/// Full client dependencies.
pub struct FullDeps<C, P, SC, BE, A: ChainApi, CT, CIDP> {
    /// The client instance to use.
    pub client: Arc<C>,
    /// Transaction pool instance.
    pub pool: Arc<P>,
    /// Whether to deny unsafe calls
    pub deny_unsafe: DenyUnsafe,
    /// Manual seal command sink
    pub command_sink: Option<mpsc::Sender<EngineCommand<Hash>>>,
    /// Ethereum-compatibility specific dependencies.
    pub eth: EthDeps<Block, C, P, A, CT, CIDP>,

    pub babe: BabeDeps,

    pub grandpa: Option<GrandpaDeps<BE>>,

    pub select_chain: SC,
}

/// Dependencies for GRANDPA
pub struct GrandpaDeps<BE> {
    /// Voting round info.
    pub shared_voter_state: sc_consensus_grandpa::SharedVoterState,
    /// Authority set info.
    pub shared_authority_set: sc_consensus_grandpa::SharedAuthoritySet<Hash, BlockNumber>,
    /// Receives notifications about justification events from Grandpa.
    pub justification_stream: sc_consensus_grandpa::GrandpaJustificationStream<Block>,
    /// Executor to drive the subscription manager in the Grandpa RPC handler.
    pub subscription_executor: sc_rpc::SubscriptionTaskExecutor,
    /// Finality proof provider.
    pub finality_provider: Arc<FinalityProofProvider<BE, Block>>,
}

pub struct BabeDeps {
    pub babe_worker: Option<BabeWorkerHandle<Block>>,
    pub keystore: sp_keystore::KeystorePtr,
}

pub struct DefaultEthConfig<C, BE>(std::marker::PhantomData<(C, BE)>);

pub struct SpawnTasksParams<'a, B: BlockT, C, BE> {
    pub task_manager: &'a TaskManager,
    pub client: Arc<C>,
    pub substrate_backend: Arc<BE>,
    pub frontier_backend: Arc<fc_db::Backend<B, C>>,
    pub filter_pool: Option<FilterPool>,
    pub overrides: Arc<OverrideHandle<B>>,
    pub fee_history_limit: u64,
    pub fee_history_cache: FeeHistoryCache,
}

pub struct TracingConfig {
    pub tracing_requesters: tracing::RpcRequesters,
    pub trace_filter_max_count: u32,
}

impl<C, BE> fc_rpc::EthConfig<Block, C> for DefaultEthConfig<C, BE>
where
    C: StorageProvider<Block, BE> + Sync + Send + 'static,
    BE: Backend<Block> + 'static,
{
    type EstimateGasAdapter = ();
    type RuntimeStorageOverride =
        fc_rpc::frontier_backend_client::SystemAccountId20StorageOverride<Block, C, BE>;
}

/// Instantiate all Full RPC extensions.
pub fn create_full<C, P, SC, BE, A, CT, CIDP>(
    deps: FullDeps<C, P, SC, BE, A, CT, CIDP>,
    subscription_task_executor: SubscriptionTaskExecutor,
    maybe_tracing_config: Option<TracingConfig>,
    pubsub_notification_sinks: Arc<
        fc_mapping_sync::EthereumBlockNotificationSinks<
            fc_mapping_sync::EthereumBlockNotification<Block>,
        >,
    >,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
    C: CallApiAt<Block> + ProvideRuntimeApi<Block>,
    C::Api: sp_block_builder::BlockBuilder<Block>,
    C::Api: sp_consensus_babe::BabeApi<Block>,
    C::Api: substrate_frame_rpc_system::AccountNonceApi<Block, AccountId, Nonce>,
    C::Api: pallet_transaction_payment_rpc::TransactionPaymentRuntimeApi<Block, Balance>,
    C::Api: fp_rpc::ConvertTransactionRuntimeApi<Block>,
    C::Api: fp_rpc::EthereumRuntimeRPCApi<Block>,
    C::Api: RuntimeApiCollection,
    C: HeaderBackend<Block> + HeaderMetadata<Block, Error = BlockChainError> + 'static,
    C: BlockchainEvents<Block> + AuxStore + UsageProvider<Block> + StorageProvider<Block, BE>,
    BE: Backend<Block> + 'static,
    BE::State: StateBackend<HasherFor<Block>>,
    P: TransactionPool<Block = Block> + 'static,
    A: ChainApi<Block = Block> + 'static,
    CIDP: CreateInherentDataProviders<Block, ()> + Send + 'static,
    CT: fp_rpc::ConvertTransaction<<Block as BlockT>::Extrinsic> + Send + Sync + 'static,
    SC: sp_consensus::SelectChain<Block> + 'static,
{
    use creditcoin3_rpc_debug::{Debug, DebugServer};
    use creditcoin3_rpc_trace::{Trace, TraceServer};
    use pallet_transaction_payment_rpc::{TransactionPayment, TransactionPaymentApiServer};
    use sc_consensus_babe_rpc::{Babe, BabeApiServer};
    use sc_consensus_grandpa_rpc::{Grandpa, GrandpaApiServer};
    use sc_consensus_manual_seal::rpc::{ManualSeal, ManualSealApiServer};
    use substrate_frame_rpc_system::{System, SystemApiServer};

    let mut io = RpcModule::new(());
    let FullDeps {
        client,
        pool,
        deny_unsafe,
        command_sink,
        eth,
        babe: BabeDeps {
            babe_worker,
            keystore,
        },
        select_chain,
        grandpa,
    } = deps;

    io.merge(System::new(Arc::clone(&client), Arc::clone(&pool), deny_unsafe).into_rpc())?;
    io.merge(TransactionPayment::new(Arc::clone(&client)).into_rpc())?;

    if let Some(command_sink) = command_sink {
        io.merge(
            // We provide the rpc handler with the sending end of the channel to allow the rpc
            // send EngineCommands to the background block authorship task.
            ManualSeal::new(command_sink).into_rpc(),
        )?;
    }

    if let Some(babe_worker) = babe_worker {
        io.merge(
            Babe::new(
                client.clone(),
                babe_worker,
                keystore,
                select_chain,
                deny_unsafe,
            )
            .into_rpc(),
        )?;
    }

    if let Some(GrandpaDeps {
        finality_provider,
        justification_stream,
        shared_authority_set,
        shared_voter_state,
        subscription_executor,
    }) = grandpa
    {
        io.merge(
            Grandpa::new(
                subscription_executor,
                shared_authority_set,
                shared_voter_state,
                justification_stream,
                finality_provider,
            )
            .into_rpc(),
        )?;
    }

    if let Some(tracing_config) = maybe_tracing_config {
        if let Some(trace_filter_requester) = tracing_config.tracing_requesters.trace {
            io.merge(
                Trace::new(
                    client.clone(),
                    trace_filter_requester,
                    tracing_config.trace_filter_max_count,
                )
                .into_rpc(),
            )?;
        }

        if let Some(debug_requester) = tracing_config.tracing_requesters.debug {
            io.merge(Debug::new(debug_requester).into_rpc())?;
        }
    }

    // Ethereum compatibility RPCs
    let io = create_eth::<_, _, _, _, _, _, _, DefaultEthConfig<C, BE>>(
        io,
        eth,
        subscription_task_executor,
        pubsub_notification_sinks,
    )?;

    Ok(io)
}
