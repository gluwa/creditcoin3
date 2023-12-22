//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

use std::{cell::RefCell, path::Path, sync::Arc, time::Duration};

use futures::{channel::mpsc, prelude::*};
// Substrate
use sc_client_api::{Backend, BlockBackend};
use sc_consensus::BasicQueue;
use sc_consensus_babe::{BabeBlockImport, BabeLink, BabeWorkerHandle};
use sc_executor::NativeExecutionDispatch;
use sc_network_sync::warp::WarpSyncParams;
use sc_service::{error::Error as ServiceError, Configuration, PartialComponents, TaskManager};
use sc_telemetry::{Telemetry, TelemetryHandle, TelemetryWorker};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use sp_api::ConstructRuntimeApi;
use sp_consensus_babe::BabeApi;
use sp_core::U256;
use substrate_prometheus_endpoint::Registry;
// Runtime
use creditcoin3_runtime::{opaque::Block, Hash, TransactionConverter};

use crate::{
    cli::Sealing,
    client::{BaseRuntimeApiCollection, FullBackend, FullClient, RuntimeApiCollection},
    eth::{
        new_frontier_partial, spawn_frontier_tasks, BackendType, EthCompatRuntimeApiCollection,
        FrontierBackend, FrontierBlockImport, FrontierPartialComponents,
    },
};
pub use crate::{
    client::{Client, TemplateRuntimeExecutor},
    eth::{db_config_dir, EthConfiguration},
};

type BasicImportQueue = sc_consensus::DefaultImportQueue<Block>;
type FullPool<Client> = sc_transaction_pool::FullPool<Block, Client>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;

type GrandpaBlockImport<Client> =
    sc_consensus_grandpa::GrandpaBlockImport<FullBackend, Block, Client, FullSelectChain>;
type GrandpaLinkHalf<Client> = sc_consensus_grandpa::LinkHalf<Block, Client, FullSelectChain>;
type BoxBlockImport = sc_consensus::BoxBlockImport<Block>;

pub struct Basics<'a, RuntimeApi, Executor: NativeExecutionDispatch + 'static>
where
    RuntimeApi:
        ConstructRuntimeApi<Block, FullClient<RuntimeApi, Executor>> + Send + Sync + 'static,
    RuntimeApi::RuntimeApi: BaseRuntimeApiCollection + EthCompatRuntimeApiCollection,
{
    telemetry: Option<TelemetryHandle>,
    task_manager: &'a TaskManager,
    client: Arc<FullClient<RuntimeApi, Executor>>,
    select_chain: FullSelectChain,
    config: &'a Configuration,
    transaction_pool: Arc<FullPool<FullClient<RuntimeApi, Executor>>>,
}

/// The minimum period of blocks on which justifications will be
/// imported and generated.
const GRANDPA_JUSTIFICATION_PERIOD: u32 = 512;

pub fn new_partial<RuntimeApi, Executor, BIQ>(
    config: &Configuration,
    eth_config: &EthConfiguration,
    build_import_queue: BIQ,
) -> Result<
    PartialComponents<
        FullClient<RuntimeApi, Executor>,
        FullBackend,
        FullSelectChain,
        BasicImportQueue,
        FullPool<FullClient<RuntimeApi, Executor>>,
        (
            Option<Telemetry>,
            BoxBlockImport,
            GrandpaLinkHalf<FullClient<RuntimeApi, Executor>>,
            BabeLink<Block>,
            FrontierBackend,
            Arc<fc_rpc::OverrideHandle<Block>>,
            Option<BabeWorkerHandle<Block>>,
        ),
    >,
    ServiceError,
>
where
    RuntimeApi: ConstructRuntimeApi<Block, FullClient<RuntimeApi, Executor>>,
    RuntimeApi: Send + Sync + 'static,
    RuntimeApi::RuntimeApi: BaseRuntimeApiCollection + EthCompatRuntimeApiCollection,
    RuntimeApi::RuntimeApi: BabeApi<Block>,
    Executor: NativeExecutionDispatch + 'static,
    BIQ: FnOnce(
        &EthConfiguration,
        Basics<RuntimeApi, Executor>,
        GrandpaBlockImport<FullClient<RuntimeApi, Executor>>,
        (
            BabeBlockImport<
                Block,
                FullClient<RuntimeApi, Executor>,
                GrandpaBlockImport<FullClient<RuntimeApi, Executor>>,
            >,
            BabeLink<Block>,
        ),
    ) -> Result<
        (
            BasicImportQueue,
            BoxBlockImport,
            Option<BabeWorkerHandle<Block>>,
        ),
        ServiceError,
    >,
{
    let telemetry = config
        .telemetry_endpoints
        .clone()
        .filter(|x| !x.is_empty())
        .map(|endpoints| -> Result<_, sc_telemetry::Error> {
            let worker = TelemetryWorker::new(16)?;
            let telemetry = worker.handle().new_telemetry(endpoints);
            Ok((worker, telemetry))
        })
        .transpose()?;

    let executor = sc_service::new_native_or_wasm_executor(config);

    let (client, backend, keystore_container, task_manager) =
        sc_service::new_full_parts::<Block, RuntimeApi, _>(
            config,
            telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
            executor,
        )?;
    let client = Arc::new(client);

    let telemetry = telemetry.map(|(worker, telemetry)| {
        task_manager
            .spawn_handle()
            .spawn("telemetry", None, worker.run());
        telemetry
    });

    let select_chain = sc_consensus::LongestChain::new(backend.clone());
    let (grandpa_block_import, grandpa_link) = sc_consensus_grandpa::block_import(
        client.clone(),
        GRANDPA_JUSTIFICATION_PERIOD,
        &client,
        select_chain.clone(),
        telemetry.as_ref().map(|x| x.handle()),
    )?;

    let overrides = crate::rpc::overrides_handle(client.clone());
    let frontier_backend = match eth_config.frontier_backend_type {
        BackendType::KeyValue => FrontierBackend::KeyValue(fc_db::kv::Backend::open(
            Arc::clone(&client),
            &config.database,
            &db_config_dir(config),
        )?),
        BackendType::Sql => {
            let db_path = db_config_dir(config).join("sql");
            std::fs::create_dir_all(&db_path).expect("failed creating sql db directory");
            let backend = futures::executor::block_on(fc_db::sql::Backend::new(
                fc_db::sql::BackendConfig::Sqlite(fc_db::sql::SqliteBackendConfig {
                    path: Path::new("sqlite:///")
                        .join(db_path)
                        .join("frontier.db3")
                        .to_str()
                        .unwrap(),
                    create_if_missing: true,
                    thread_count: eth_config.frontier_sql_backend_thread_count,
                    cache_size: eth_config.frontier_sql_backend_cache_size,
                }),
                eth_config.frontier_sql_backend_pool_size,
                std::num::NonZeroU32::new(eth_config.frontier_sql_backend_num_ops_timeout),
                overrides.clone(),
            ))
            .unwrap_or_else(|err| panic!("failed creating sql backend: {:?}", err));
            FrontierBackend::Sql(backend)
        }
    };

    let babe_config = sc_consensus_babe::configuration(&*client)?;
    let (babe_block_import, babe_link) =
        sc_consensus_babe::block_import(babe_config, grandpa_block_import.clone(), client.clone())?;

    let transaction_pool = sc_transaction_pool::BasicPool::new_full(
        config.transaction_pool.clone(),
        config.role.is_authority().into(),
        config.prometheus_registry(),
        task_manager.spawn_essential_handle(),
        client.clone(),
    );

    let (import_queue, block_import, babe_worker) = build_import_queue(
        eth_config,
        Basics {
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            task_manager: &task_manager,
            client: client.clone(),
            select_chain: select_chain.clone(),
            config,
            transaction_pool: transaction_pool.clone(),
        },
        grandpa_block_import,
        (babe_block_import, babe_link.clone()),
    )?;

    Ok(PartialComponents {
        client,
        backend,
        keystore_container,
        task_manager,
        select_chain,
        import_queue,
        transaction_pool,
        other: (
            telemetry,
            block_import,
            grandpa_link,
            babe_link,
            frontier_backend,
            overrides,
            babe_worker,
        ),
    })
}

/// Build the import queue for the runtime (babe + grandpa).
pub fn build_babe_grandpa_import_queue<RuntimeApi, Executor>(
    eth_config: &EthConfiguration,
    basics: Basics<RuntimeApi, Executor>,
    grandpa_block_import: GrandpaBlockImport<FullClient<RuntimeApi, Executor>>,
    babe_import: (
        BabeBlockImport<
            Block,
            FullClient<RuntimeApi, Executor>,
            GrandpaBlockImport<FullClient<RuntimeApi, Executor>>,
        >,
        BabeLink<Block>,
    ),
) -> Result<
    (
        BasicImportQueue,
        BoxBlockImport,
        Option<BabeWorkerHandle<Block>>,
    ),
    ServiceError,
>
where
    RuntimeApi: ConstructRuntimeApi<Block, FullClient<RuntimeApi, Executor>>,
    RuntimeApi: Send + Sync + 'static,
    RuntimeApi::RuntimeApi: RuntimeApiCollection,
    Executor: NativeExecutionDispatch + 'static,
{
    let Basics {
        telemetry,
        task_manager,
        client,
        select_chain,
        config,
        transaction_pool,
    } = basics;
    let (babe_import, babe_link) = babe_import;
    let slot_duration = babe_link.config().slot_duration();
    let target_gas_price = eth_config.target_gas_price;
    let create_inherent_data_providers = move |_parent, ()| async move {
        let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

        let slot =
            sp_consensus_babe::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
                *timestamp,
                slot_duration,
            );

        let dynamic_fee: fp_dynamic_fee::InherentDataProvider =
            fp_dynamic_fee::InherentDataProvider(U256::from(target_gas_price));
        Ok((slot, timestamp, dynamic_fee))
    };

    let frontier_block_import = FrontierBlockImport::new(babe_import, client.clone());

    let (import_queue, babe_worker) =
        sc_consensus_babe::import_queue(sc_consensus_babe::ImportQueueParams {
            link: babe_link,
            block_import: frontier_block_import.clone(),
            justification_import: Some(Box::new(grandpa_block_import)),
            client,
            select_chain,
            create_inherent_data_providers,
            spawner: &task_manager.spawn_essential_handle(),
            registry: config.prometheus_registry(),
            telemetry: telemetry.as_ref().cloned(),
            offchain_tx_pool_factory: OffchainTransactionPoolFactory::new(transaction_pool),
        })?;

    Ok((
        import_queue,
        Box::new(frontier_block_import),
        Some(babe_worker),
    ))
}

/// Build the import queue for the template runtime (manual seal).
pub fn build_manual_seal_import_queue<RuntimeApi, Executor>(
    _eth_config: &EthConfiguration,
    basics: Basics<RuntimeApi, Executor>,
    _grandpa_block_import: GrandpaBlockImport<FullClient<RuntimeApi, Executor>>,
    _babe_import: (
        BabeBlockImport<
            Block,
            FullClient<RuntimeApi, Executor>,
            GrandpaBlockImport<FullClient<RuntimeApi, Executor>>,
        >,
        BabeLink<Block>,
    ),
) -> Result<
    (
        BasicImportQueue,
        BoxBlockImport,
        Option<BabeWorkerHandle<Block>>,
    ),
    ServiceError,
>
where
    RuntimeApi: ConstructRuntimeApi<Block, FullClient<RuntimeApi, Executor>>,
    RuntimeApi: Send + Sync + 'static,
    RuntimeApi::RuntimeApi: RuntimeApiCollection,
    Executor: NativeExecutionDispatch + 'static,
{
    let Basics {
        task_manager,
        client,
        config,
        ..
    } = basics;
    let frontier_block_import = FrontierBlockImport::new(client.clone(), client);
    Ok((
        sc_consensus_manual_seal::import_queue(
            Box::new(frontier_block_import.clone()),
            &task_manager.spawn_essential_handle(),
            config.prometheus_registry(),
        ),
        Box::new(frontier_block_import),
        None,
    ))
}

/// Builds a new service for a full client.
pub async fn new_full<RuntimeApi, Executor>(
    mut config: Configuration,
    eth_config: EthConfiguration,
    sealing: Option<Sealing>,
) -> Result<TaskManager, ServiceError>
where
    RuntimeApi: ConstructRuntimeApi<Block, FullClient<RuntimeApi, Executor>>,
    RuntimeApi: Send + Sync + 'static,
    RuntimeApi::RuntimeApi: RuntimeApiCollection,
    Executor: NativeExecutionDispatch + 'static,
{
    let build_import_queue = if sealing.is_some() {
        build_manual_seal_import_queue::<RuntimeApi, Executor>
    } else {
        build_babe_grandpa_import_queue::<RuntimeApi, Executor>
    };

    let PartialComponents {
        client,
        backend,
        mut task_manager,
        import_queue,
        keystore_container,
        select_chain,
        transaction_pool,
        other:
            (
                mut telemetry,
                block_import,
                grandpa_link,
                babe_link,
                frontier_backend,
                overrides,
                babe_worker,
            ),
    } = new_partial(&config, &eth_config, build_import_queue)?;

    let FrontierPartialComponents {
        filter_pool,
        fee_history_cache,
        fee_history_cache_limit,
    } = new_frontier_partial(&eth_config)?;

    let mut net_config = sc_network::config::FullNetworkConfiguration::new(&config.network);
    let grandpa_protocol_name = sc_consensus_grandpa::protocol_standard_name(
        &client.block_hash(0)?.expect("Genesis block exists; qed"),
        &config.chain_spec,
    );

    let warp_sync_params = if sealing.is_some() {
        None
    } else {
        net_config.add_notification_protocol(sc_consensus_grandpa::grandpa_peers_set_config(
            grandpa_protocol_name.clone(),
        ));
        let warp_sync: Arc<dyn sc_network::config::WarpSyncProvider<Block>> =
            Arc::new(sc_consensus_grandpa::warp_proof::NetworkProvider::new(
                backend.clone(),
                grandpa_link.shared_authority_set().clone(),
                Vec::default(),
            ));
        Some(WarpSyncParams::WithProvider(warp_sync))
    };

    let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
        sc_service::build_network(sc_service::BuildNetworkParams {
            config: &config,
            net_config,
            client: client.clone(),
            transaction_pool: transaction_pool.clone(),
            spawn_handle: task_manager.spawn_handle(),
            import_queue,
            block_announce_validator_builder: None,
            warp_sync_params,
        })?;

    if config.offchain_worker.enabled {
        task_manager.spawn_handle().spawn(
            "offchain-workers-runner",
            "offchain-worker",
            sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
                runtime_api_provider: client.clone(),
                is_validator: config.role.is_authority(),
                keystore: Some(keystore_container.keystore()),
                offchain_db: backend.offchain_storage(),
                transaction_pool: Some(OffchainTransactionPoolFactory::new(
                    transaction_pool.clone(),
                )),
                network_provider: network.clone(),
                enable_http_requests: true,
                custom_extensions: |_| vec![],
            })
            .run(client.clone(), task_manager.spawn_handle())
            .boxed(),
        );
    }

    let role = config.role.clone();
    let force_authoring = config.force_authoring;
    let name = config.network.node_name.clone();
    let enable_grandpa = !config.disable_grandpa && sealing.is_none();
    let prometheus_registry = config.prometheus_registry().cloned();

    // Channel for the rpc handler to communicate with the authorship task.
    let (command_sink, commands_stream) = mpsc::channel(1000);

    // Sinks for pubsub notifications.
    // Everytime a new subscription is created, a new mpsc channel is added to the sink pool.
    // The MappingSyncWorker sends through the channel on block import and the subscription emits a notification to the subscriber on receiving a message through this channel.
    // This way we avoid race conditions when using native substrate block import notification stream.
    let pubsub_notification_sinks: fc_mapping_sync::EthereumBlockNotificationSinks<
        fc_mapping_sync::EthereumBlockNotification<Block>,
    > = Default::default();
    let pubsub_notification_sinks = Arc::new(pubsub_notification_sinks);

    // for ethereum-compatibility rpc.
    config.rpc_id_provider = Some(Box::new(fc_rpc::EthereumSubIdProvider));

    let shared_voter_state = sc_consensus_grandpa::SharedVoterState::empty();

    let rpc_builder = {
        let client = client.clone();
        let pool = transaction_pool.clone();
        let network = network.clone();
        let sync_service = sync_service.clone();

        let is_authority = role.is_authority();
        let enable_dev_signer = eth_config.enable_dev_signer;
        let max_past_logs = eth_config.max_past_logs;
        let execute_gas_limit_multiplier = eth_config.execute_gas_limit_multiplier;
        let filter_pool = filter_pool.clone();
        let frontier_backend = frontier_backend.clone();
        let pubsub_notification_sinks = pubsub_notification_sinks.clone();
        let overrides = overrides.clone();
        let fee_history_cache = fee_history_cache.clone();
        let block_data_cache = Arc::new(fc_rpc::EthBlockDataCacheTask::new(
            task_manager.spawn_handle(),
            overrides.clone(),
            eth_config.eth_log_block_cache,
            eth_config.eth_statuses_cache,
            prometheus_registry.clone(),
        ));

        let slot_duration = sc_consensus_babe::configuration(&*client)?.slot_duration();
        let target_gas_price = eth_config.target_gas_price;
        let pending_create_inherent_data_providers = move |_, ()| async move {
            let current = sp_timestamp::InherentDataProvider::from_system_time();
            let next_slot = current.timestamp().as_millis() + slot_duration.as_millis();
            let timestamp = sp_timestamp::InherentDataProvider::new(next_slot.into());
            let slot = sp_consensus_babe::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
				*timestamp,
				slot_duration,
			);
            let dynamic_fee = fp_dynamic_fee::InherentDataProvider(U256::from(target_gas_price));
            Ok((slot, timestamp, dynamic_fee))
        };
        let select_chain = select_chain.clone();
        let keystore = keystore_container.keystore();

        let shared_authority_set = grandpa_link.shared_authority_set().clone();
        let finality_provider = sc_consensus_grandpa::FinalityProofProvider::new_for_service(
            backend.clone(),
            Some(shared_authority_set.clone()),
        );
        let justification_stream = grandpa_link.justification_stream();
        let shared_voter_state = shared_voter_state.clone();

        Box::new(
            move |deny_unsafe, subscription_task_executor: sc_rpc::SubscriptionTaskExecutor| {
                let eth_deps = crate::rpc::EthDeps {
                    client: client.clone(),
                    pool: pool.clone(),
                    graph: pool.pool().clone(),
                    converter: Some(TransactionConverter),
                    is_authority,
                    enable_dev_signer,
                    network: network.clone(),
                    sync: sync_service.clone(),
                    frontier_backend: match frontier_backend.clone() {
                        fc_db::Backend::KeyValue(b) => Arc::new(b),
                        fc_db::Backend::Sql(b) => Arc::new(b),
                    },
                    overrides: overrides.clone(),
                    block_data_cache: block_data_cache.clone(),
                    filter_pool: filter_pool.clone(),
                    max_past_logs,
                    fee_history_cache: fee_history_cache.clone(),
                    fee_history_cache_limit,
                    execute_gas_limit_multiplier,
                    forced_parent_hashes: None,
                    pending_create_inherent_data_providers,
                    pending_consensus_data_provider: Some(
                        crate::rpc::BabeConsensusDataProvider::new(),
                    ),
                };
                let deps = crate::rpc::FullDeps {
                    client: client.clone(),
                    pool: pool.clone(),
                    deny_unsafe,
                    command_sink: if sealing.is_some() {
                        Some(command_sink.clone())
                    } else {
                        None
                    },
                    eth: eth_deps,
                    babe: crate::rpc::BabeDeps {
                        babe_worker: babe_worker.clone(),
                        keystore: keystore.clone(),
                    },
                    select_chain: select_chain.clone(),
                    grandpa: enable_grandpa.then(|| crate::rpc::GrandpaDeps {
                        finality_provider: finality_provider.clone(),
                        justification_stream: justification_stream.clone(),
                        shared_authority_set: shared_authority_set.clone(),
                        shared_voter_state: shared_voter_state.clone(),
                        subscription_executor: subscription_task_executor.clone(),
                    }),
                };
                crate::rpc::create_full(
                    deps,
                    subscription_task_executor,
                    pubsub_notification_sinks.clone(),
                )
                .map_err(Into::into)
            },
        )
    };

    let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
        config,
        client: client.clone(),
        backend: backend.clone(),
        task_manager: &mut task_manager,
        keystore: keystore_container.keystore(),
        transaction_pool: transaction_pool.clone(),
        rpc_builder,
        network: network.clone(),
        system_rpc_tx,
        tx_handler_controller,
        sync_service: sync_service.clone(),
        telemetry: telemetry.as_mut(),
    })?;

    spawn_frontier_tasks(
        &task_manager,
        client.clone(),
        backend,
        frontier_backend,
        filter_pool,
        overrides,
        fee_history_cache,
        fee_history_cache_limit,
        sync_service.clone(),
        pubsub_notification_sinks,
    )
    .await;

    if role.is_authority() {
        // manual-seal authorship
        if let Some(sealing) = sealing {
            run_manual_seal_authorship(
                &eth_config,
                sealing,
                client,
                transaction_pool,
                select_chain,
                block_import,
                &task_manager,
                prometheus_registry.as_ref(),
                telemetry.as_ref(),
                commands_stream,
            )?;

            network_starter.start_network();
            log::info!("Manual Seal Ready");
            return Ok(task_manager);
        }

        let proposer_factory = sc_basic_authorship::ProposerFactory::new(
            task_manager.spawn_handle(),
            client.clone(),
            transaction_pool.clone(),
            prometheus_registry.as_ref(),
            telemetry.as_ref().map(|x| x.handle()),
        );

        let slot_duration = sc_consensus_babe::configuration(&*client)?.slot_duration();
        let target_gas_price = eth_config.target_gas_price;

        let create_inherent_data_providers = move |_parent, ()| async move {
            let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

            let slot =
                sp_consensus_babe::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
                    *timestamp,
                    slot_duration,
                );

            let dynamic_fee: fp_dynamic_fee::InherentDataProvider =
                fp_dynamic_fee::InherentDataProvider(U256::from(target_gas_price));
            Ok((slot, timestamp, dynamic_fee))
        };

        let babe = sc_consensus_babe::start_babe(sc_consensus_babe::BabeParams {
            keystore: keystore_container.keystore(),
            client,
            select_chain,
            env: proposer_factory,
            block_import,
            sync_oracle: sync_service.clone(),
            justification_sync_link: sync_service.clone(),
            create_inherent_data_providers,
            force_authoring,
            backoff_authoring_blocks: None::<()>,
            babe_link,
            block_proposal_slot_portion: sc_consensus_babe::SlotProportion::new(2f32 / 3f32),
            max_block_proposal_slot_portion: None,
            telemetry: telemetry.as_ref().map(|x| x.handle()),
        })?;
        // the BABE authoring task is considered essential, i.e. if it
        // fails we take down the service with it.
        task_manager
            .spawn_essential_handle()
            .spawn_blocking("babe", Some("block-authoring"), babe);
    }

    if enable_grandpa {
        // if the node isn't actively participating in consensus then it doesn't
        // need a keystore, regardless of which protocol we use below.
        let keystore = if role.is_authority() {
            Some(keystore_container.keystore())
        } else {
            None
        };

        let grandpa_config = sc_consensus_grandpa::Config {
            // FIXME #1578 make this available through chainspec
            gossip_duration: Duration::from_millis(333),
            justification_generation_period: GRANDPA_JUSTIFICATION_PERIOD,
            name: Some(name),
            observer_enabled: false,
            keystore,
            local_role: role,
            telemetry: telemetry.as_ref().map(|x| x.handle()),
            protocol_name: grandpa_protocol_name,
        };

        // start the full GRANDPA voter
        // NOTE: non-authorities could run the GRANDPA observer protocol, but at
        // this point the full voter should provide better guarantees of block
        // and vote data availability than the observer. The observer has not
        // been tested extensively yet and having most nodes in a network run it
        // could lead to finality stalls.
        let grandpa_voter =
            sc_consensus_grandpa::run_grandpa_voter(sc_consensus_grandpa::GrandpaParams {
                config: grandpa_config,
                link: grandpa_link,
                network,
                sync: sync_service,
                voting_rule: sc_consensus_grandpa::VotingRulesBuilder::default().build(),
                prometheus_registry,
                shared_voter_state,
                telemetry: telemetry.as_ref().map(|x| x.handle()),
                offchain_tx_pool_factory: OffchainTransactionPoolFactory::new(transaction_pool),
            })?;

        // the GRANDPA voter task is considered infallible, i.e.
        // if it fails we take down the service with it.
        task_manager
            .spawn_essential_handle()
            .spawn_blocking("grandpa-voter", None, grandpa_voter);
    }

    network_starter.start_network();
    Ok(task_manager)
}

fn run_manual_seal_authorship<RuntimeApi, Executor>(
    eth_config: &EthConfiguration,
    sealing: Sealing,
    client: Arc<FullClient<RuntimeApi, Executor>>,
    transaction_pool: Arc<FullPool<FullClient<RuntimeApi, Executor>>>,
    select_chain: FullSelectChain,
    block_import: BoxBlockImport,
    task_manager: &TaskManager,
    prometheus_registry: Option<&Registry>,
    telemetry: Option<&Telemetry>,
    commands_stream: mpsc::Receiver<sc_consensus_manual_seal::rpc::EngineCommand<Hash>>,
) -> Result<(), ServiceError>
where
    RuntimeApi: ConstructRuntimeApi<Block, FullClient<RuntimeApi, Executor>>,
    RuntimeApi: Send + Sync + 'static,
    RuntimeApi::RuntimeApi: RuntimeApiCollection,
    Executor: NativeExecutionDispatch + 'static,
{
    let proposer_factory = sc_basic_authorship::ProposerFactory::new(
        task_manager.spawn_handle(),
        client.clone(),
        transaction_pool.clone(),
        prometheus_registry,
        telemetry.as_ref().map(|x| x.handle()),
    );

    thread_local!(static TIMESTAMP: RefCell<u64> = RefCell::new(0));

    /// Provide a mock duration starting at 0 in millisecond for timestamp inherent.
    /// Each call will increment timestamp by slot_duration making Babe think time has passed.
    struct MockTimestampInherentDataProvider;

    #[async_trait::async_trait]
    impl sp_inherents::InherentDataProvider for MockTimestampInherentDataProvider {
        async fn provide_inherent_data(
            &self,
            inherent_data: &mut sp_inherents::InherentData,
        ) -> Result<(), sp_inherents::Error> {
            TIMESTAMP.with(|x| {
                *x.borrow_mut() += creditcoin3_runtime::SLOT_DURATION;
                inherent_data.put_data(sp_timestamp::INHERENT_IDENTIFIER, &*x.borrow())
            })
        }

        async fn try_handle_error(
            &self,
            _identifier: &sp_inherents::InherentIdentifier,
            _error: &[u8],
        ) -> Option<Result<(), sp_inherents::Error>> {
            // The pallet never reports error.
            None
        }
    }

    let target_gas_price = eth_config.target_gas_price;
    let create_inherent_data_providers = move |_, ()| async move {
        let timestamp = MockTimestampInherentDataProvider;
        let dynamic_fee = fp_dynamic_fee::InherentDataProvider(U256::from(target_gas_price));
        Ok((timestamp, dynamic_fee))
    };

    let manual_seal = match sealing {
        Sealing::Manual => future::Either::Left(sc_consensus_manual_seal::run_manual_seal(
            sc_consensus_manual_seal::ManualSealParams {
                block_import,
                env: proposer_factory,
                client,
                pool: transaction_pool,
                commands_stream,
                select_chain,
                consensus_data_provider: None,
                create_inherent_data_providers,
            },
        )),
        Sealing::Instant => future::Either::Right(sc_consensus_manual_seal::run_instant_seal(
            sc_consensus_manual_seal::InstantSealParams {
                block_import,
                env: proposer_factory,
                client,
                pool: transaction_pool,
                select_chain,
                consensus_data_provider: None,
                create_inherent_data_providers,
            },
        )),
    };

    // we spawn the future on a background thread managed by service.
    task_manager
        .spawn_essential_handle()
        .spawn_blocking("manual-seal", None, manual_seal);
    Ok(())
}

pub async fn build_full(
    config: Configuration,
    eth_config: EthConfiguration,
    sealing: Option<Sealing>,
) -> Result<TaskManager, ServiceError> {
    new_full::<creditcoin3_runtime::RuntimeApi, TemplateRuntimeExecutor>(
        config, eth_config, sealing,
    )
    .await
}

pub fn new_chain_ops(
    config: &mut Configuration,
    eth_config: &EthConfiguration,
) -> Result<
    (
        Arc<Client>,
        Arc<FullBackend>,
        BasicQueue<Block>,
        TaskManager,
        FrontierBackend,
    ),
    ServiceError,
> {
    config.keystore = sc_service::config::KeystoreConfig::InMemory;
    let PartialComponents {
        client,
        backend,
        import_queue,
        task_manager,
        other,
        ..
    } = new_partial::<creditcoin3_runtime::RuntimeApi, TemplateRuntimeExecutor, _>(
        config,
        eth_config,
        build_babe_grandpa_import_queue,
    )?;
    Ok((client, backend, import_queue, task_manager, other.4))
}
