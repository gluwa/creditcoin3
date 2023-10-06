//! A collection of node-specific RPC methods.

use std::sync::Arc;

use futures::channel::mpsc;
use jsonrpsee::RpcModule;
// Substrate
use sc_client_api::{
    backend::{Backend, StorageProvider},
    client::BlockchainEvents,
    AuxStore, UsageProvider,
};
use sc_consensus_babe::BabeWorkerHandle;
use sc_consensus_manual_seal::rpc::EngineCommand;
use sc_rpc::SubscriptionTaskExecutor;
use sc_rpc_api::DenyUnsafe;
use sc_service::TransactionPool;
use sc_transaction_pool::ChainApi;
use sp_api::{CallApiAt, ProvideRuntimeApi};
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::Block as BlockT;
// Runtime
use frontier_template_runtime::{opaque::Block, AccountId, Balance, Hash, Nonce};

mod eth;
pub use self::eth::{
    consensus_data_provider::{self, BabeConsensusDataProvider},
    create_eth, overrides_handle, EthDeps,
};

/// Full client dependencies.
pub struct FullDeps<C, P, SC, A: ChainApi, CT, CIDP> {
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

    pub select_chain: SC,
}

pub struct BabeDeps {
    pub babe_worker: Option<BabeWorkerHandle<Block>>,
    pub keystore: sp_keystore::KeystorePtr,
}

pub struct DefaultEthConfig<C, BE>(std::marker::PhantomData<(C, BE)>);

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
    deps: FullDeps<C, P, SC, A, CT, CIDP>,
    subscription_task_executor: SubscriptionTaskExecutor,
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
    P: TransactionPool<Block = Block> + 'static,
    A: ChainApi<Block = Block> + 'static,
    CIDP: CreateInherentDataProviders<Block, ()> + Send + 'static,
    CT: fp_rpc::ConvertTransaction<<Block as BlockT>::Extrinsic> + Send + Sync + 'static,
    SC: sp_consensus::SelectChain<Block> + 'static,
{
    use pallet_transaction_payment_rpc::{TransactionPayment, TransactionPaymentApiServer};
    use sc_consensus_babe_rpc::{Babe, BabeApiServer};
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
    } = deps;

    io.merge(System::new(client.clone(), pool, deny_unsafe).into_rpc())?;
    io.merge(TransactionPayment::new(client.clone()).into_rpc())?;

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
                babe_worker.clone(),
                keystore,
                select_chain,
                deny_unsafe,
            )
            .into_rpc(),
        )?;
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
