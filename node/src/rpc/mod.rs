//! A collection of node-specific RPC methods.

pub mod tracing;

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
use creditcoin3_runtime::{opaque::Block, AccountId, Balance, BlockNumber, Hash, Nonce};
use fc_rpc::{
    pending::ConsensusDataProvider, EthBlockDataCacheTask, EthTask, OverrideHandle,
    RuntimeApiStorageOverride, SchemaV1Override, SchemaV2Override, SchemaV3Override,
    StorageOverride,
};
use fc_rpc_core::types::{CallRequest, FeeHistoryCache, FilterPool};
use moonbeam_cli_opt::EthApi as EthApiCmd;
use sc_transaction_pool::Pool;

mod eth;

pub use self::eth::{
    consensus_data_provider::{self, BabeConsensusDataProvider},
    create_eth, overrides_handle, EthDeps,
};

type HasherFor<Block> = <<Block as BlockT>::Header as HeaderT>::Hashing;

pub struct MoonbeamEGA;

impl fc_rpc::EstimateGasAdapter for MoonbeamEGA {
    fn adapt_request(mut request: CallRequest) -> CallRequest {
        // Redirect any call to batch precompile:
        // force usage of batchAll method for estimation
        use sp_core::H160;
        const BATCH_PRECOMPILE_ADDRESS: H160 = H160(hex_literal::hex!(
            "0000000000000000000000000000000000000808"
        ));
        const BATCH_PRECOMPILE_BATCH_ALL_SELECTOR: [u8; 4] = hex_literal::hex!("96e292b8");
        if request.to == Some(BATCH_PRECOMPILE_ADDRESS) {
            if let Some(ref mut data) = request.data {
                if data.0.len() >= 4 {
                    data.0[..4].copy_from_slice(&BATCH_PRECOMPILE_BATCH_ALL_SELECTOR);
                }
            }
        }
        request
    }
}

pub struct MoonbeamEthConfig<C, BE>(std::marker::PhantomData<(C, BE)>);

impl<C, BE> fc_rpc::EthConfig<Block, C> for MoonbeamEthConfig<C, BE>
where
    C: sc_client_api::StorageProvider<Block, BE> + Sync + Send + 'static,
    BE: Backend<Block> + 'static,
{
    type EstimateGasAdapter = MoonbeamEGA;
    type RuntimeStorageOverride =
        fc_rpc::frontier_backend_client::SystemAccountId20StorageOverride<Block, C, BE>;
}

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
    /// EthFilterApi pool.
    pub filter_pool: Option<FilterPool>,
    /// The list of optional RPC extensions.
    pub ethapi_cmd: Vec<EthApiCmd>,
    /// Ethereum data access overrides.
    pub overrides: Arc<OverrideHandle<Block>>,
    /// Cache for Ethereum block data.
    pub block_data_cache: Arc<EthBlockDataCacheTask<Block>>,
    /// Maximum number of logs in a query.
    pub max_past_logs: u32,
    /// Maximum fee history cache size.
    pub fee_history_limit: u64,
    /// Fee history cache.
    pub fee_history_cache: FeeHistoryCache,
    /// Frontier Backend.
    pub frontier_backend: Arc<dyn fc_api::Backend<Block>>,
    /// Backend.
    pub backend: Arc<BE>,
    /// Graph pool instance.
    pub graph: Arc<Pool<A>>,
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
    pub frontier_backend: fc_db::Backend<B>,
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
    type EstimateGasAdapter = MoonbeamEGA;
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
    use fc_rpc::{
        Eth, EthApiServer, EthFilter, EthFilterApiServer, EthPubSub, EthPubSubApiServer, Net,
        NetApiServer, Web3, Web3ApiServer,
    };
    use moonbeam_finality_rpc::{MoonbeamFinality, MoonbeamFinalityApiServer};
    use moonbeam_rpc_debug::{Debug, DebugServer};
    use moonbeam_rpc_trace::{Trace, TraceServer};
    use moonbeam_rpc_txpool::{TxPool, TxPoolServer};
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
        filter_pool,
        ethapi_cmd,
        overrides,
        block_data_cache,
        max_past_logs,
        fee_history_limit,
        fee_history_cache,
        frontier_backend,
        backend,
        graph,
    } = deps;

    io.merge(System::new(Arc::clone(&client), Arc::clone(&pool), deny_unsafe).into_rpc())?;
    io.merge(TransactionPayment::new(Arc::clone(&client)).into_rpc())?;

    // if let Some(filter_pool) = filter_pool {
    //     io.merge(
    //         EthFilter::new(
    //             client.clone(),
    //             frontier_backend.clone(),
    //             graph.clone(),
    //             filter_pool,
    //             500_usize, // max stored filters
    //             max_past_logs,
    //             block_data_cache,
    //         )
    //         .into_rpc(),
    //     )?;
    // }

    if let Some(command_sink) = command_sink {
        io.merge(
            // We provide the rpc handler with the sending end of the channel to allow the rpc
            // send EngineCommands to the background block authorship task.
            ManualSeal::new(command_sink).into_rpc(),
        )?;
    }

    if let Some(babe_worker) = babe_worker {
        io.merge(Babe::new(client, babe_worker, keystore, select_chain, deny_unsafe).into_rpc())?;
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

    // if ethapi_cmd.contains(&EthApiCmd::Txpool) {
    //     io.merge(TxPool::new(Arc::clone(&client), graph).into_rpc())?;
    // }

    if let Some(tracing_config) = maybe_tracing_config {
        if let Some(trace_filter_requester) = tracing_config.tracing_requesters.trace {
            io.merge(
                Trace::new(
                    client,
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
