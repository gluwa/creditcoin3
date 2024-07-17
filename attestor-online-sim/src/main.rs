mod attestation_blocks_builder_instance;
mod fragment_manager;
mod fragment_manager_instance;
mod historical_blocks_crawler_instance;
mod historical_blocks_fragment_manager_instance;

use crate::attestation_blocks_builder_instance::*;
use crate::fragment_manager_instance::*;
use crate::historical_blocks_crawler_instance::{
    create_historical_blocks_crawler_instance, InstanceCreationError,
};
use attestation_blocks_online_builder::AttestationChainOnlineBuilder;
use attestation_blocks_online_builder::SOURCE_BLOCK_TIME_MILLIS;
use attestation_chain::attestation_checkpoints_for_dev::AttestationCheckpointsForDev;
use clap::Parser;
use colored::Colorize;
use utils::json_serializable::JsonSerializable;
//use common::poc_config::{AttestationBlocksBuilderConfig, PocConfig};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::signal;
use tokio::sync::mpsc::channel;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use ethereum_types::U256;
use poc_config::{PocConfig, AttestationBlocksBuilderConfig};

fn print_with_timestamp(s: colored::ColoredString) {
    println!(
        "[{}] {}",
        chrono::Local::now().time().format("%H:%M:%S%.3f"),
        s
    );
}

//pub trait OnBlockStopConditionPredicate = Fn(u64) -> bool + Send + Sync + 'static;

#[derive(Clone)]
pub enum StopCondition {
    FatalFailure,
    OnBlockReached(Arc<dyn Fn(U256) -> bool + Send + Sync + 'static>),
    //    OnBlockReached(Arc<dyn OnBlockStopConditionPredicate>),
}
impl Default for StopCondition {
    fn default() -> Self {
        Self::FatalFailure
    }
}

#[derive(Parser, Debug)]
#[command()]
struct Args {
    #[arg(long)]
    config_file: Option<String>,

    #[arg(long)]
    from_block: Option<u64>,
}

fn main() {
    let args = Args::parse();

    let poc_config = args
        .config_file
        .map(|config_file| {
            println!("configuration file: {config_file}");
            PocConfig::try_from_file(&config_file)
        })
        .unwrap_or({
            println!(
                "configuration file not specified, defaulted to: {}",
                PocConfig::default_file()
            );
            PocConfig::try_default()
        });

    let poc_config = match poc_config {
        Ok(poc_config) => poc_config,
        Err(err) => {
            println!(
                "can't parse PoC configuration, check {}, error: {err:?}",
                PocConfig::default_file()
            );
            return;
        }
    };
    if poc_config.attestation_blocks_builder().is_none() {
        println!(
            "{}",
            "attestor config section not present, set it as described in config_template.json"
                .red()
        );
        return;
    }
    let attestor_config = poc_config
        .attestation_blocks_builder()
        .expect("checked for some");

    let source_chain_api_server_url = poc_config.source_chain_api_server_url();
    let block_cache_dir = poc_config.block_cache_url();
    let wss_url = attestor_config.source_chain_wss_server_url();
    let max_num_of_blocks_to_retrieve = attestor_config
        .max_num_of_blocks_to_retrieve()
        .unwrap_or(AttestationBlocksBuilderConfig::default_max_num_of_blocks_to_retrieve());
    let checkpoints_path = &match poc_config.execution_chain_url() {
        Some(checkpoints_path) => checkpoints_path.to_owned(),
        None => {
            println!("execution chain url, setting default value");
            PocConfig::default_execution_chain_url()
        }
    };

    if let Some(cache_dir) = block_cache_dir.as_ref() {
        println!(
            "{}",
            format!("NOTE: cache mode is on, blocks will be saved to {cache_dir}")
                .yellow()
                .bold()
        );
        if let Ok(cache_dir_size) = fs_extra::dir::get_size(cache_dir) {
            println!(
                "{}",
                format!("block cache size: {cache_dir_size}")
                    .yellow()
                    .bold()
            );
        }
    }

    let core_ids = core_affinity::get_core_ids().unwrap();

    println!("cpu cores: {}", core_ids.len());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_io()
        .enable_time()
        .worker_threads(core_ids.len())
        .on_thread_start(|| {
            println!("{}", "tokio thread started".bold().blue());
        })
        .build()
        .unwrap();

    println!("{}", "press Ctrl+C to quit".bold());
    println!("{}\n{}", 
        format!("backpressure parameter (max number of blocks to remotely retrieve in parallel): {}",
                max_num_of_blocks_to_retrieve
        ).bold(),
        "the higher this parameter is, more chances to keep up with block production rate and higher probability of getting DoS errors"
            .to_owned().bold(),
    );

    let runtime = Arc::new(runtime);

    let checkpoints = Arc::new(RwLock::new(
        AttestationCheckpointsForDev::with_execution_chain_url(checkpoints_path),
    ));

    let cancel_on_fatal_failure = CancellationToken::new();
    let cancel_on_fragment_manager_dead_cloned = cancel_on_fatal_failure.clone();

    let (attestation_chain_builder, block_receiver) = match args.from_block {
        None => create_attestation_blocks_builder_instance(
            source_chain_api_server_url,
            wss_url,
            block_cache_dir,
            Arc::clone(&runtime),
            max_num_of_blocks_to_retrieve,
            cancel_on_fatal_failure.clone(),
        ),
        Some(from_block) => {
            let (crawler_kickoff_fragment_tx, crawler_kickoff_fragment_rx) = channel(1);
            let _ = runtime.block_on(crawler_kickoff_fragment_tx.send(from_block));

            match runtime.block_on(create_historical_blocks_crawler_instance(
                source_chain_api_server_url,
                block_cache_dir,
                Arc::clone(&runtime),
                max_num_of_blocks_to_retrieve,
                cancel_on_fatal_failure.clone(),
                crawler_kickoff_fragment_rx,
                StopCondition::FatalFailure,
            )) {
                Ok((historical_blocks_crawler, db_block_receiver)) => {
                    (historical_blocks_crawler, db_block_receiver)
                }
                Err(InstanceCreationError::Other(msg)) => {
                    println!(
                        "{}",
                        format!("historical blocks crawler could not start: {}", msg).red()
                    );
                    return;
                }
                Err(err) => {
                    println!(
                        "{}",
                        format!("historical blocks crawler could not start: {:?}", err).red()
                    );
                    return;
                }
            }
        }
    };

    let (fragment_manager, _crawler_kickoff_block_rx) = create_fragment_manager_instance(
        Arc::clone(&runtime),
        Arc::clone(&checkpoints),
        block_receiver,
        cancel_on_fatal_failure.clone(),
    );
    runtime.block_on(async {
        tokio::select! {
            _ = signal::ctrl_c() => (),
            _ = cancel_on_fragment_manager_dead_cloned.cancelled() => (),
        }
    });
    shutdown_instance(attestation_chain_builder, Arc::clone(&runtime));

    println!(
        "{}",
        "main task is waiting for fragment manager to exit...".yellow()
    );
    let _ = runtime.block_on(fragment_manager.shutdown());

    println!("{}", "main task is exitting".yellow());

    let mut checkpoints = AttestationCheckpointsForDev::with_execution_chain_url(checkpoints_path);
    match checkpoints.poll() {
        Ok(_) => {
            println!("{}", 
                format!("\ncheckpoints generated:\n[tail@{:?} <---...--> stabilized head@{:?} <--> recent head@{:?}]", 
                    checkpoints.inner().tail(), checkpoints.inner().stabilized_head(), checkpoints.inner().head()
                ).bold()
            );
            if checkpoints.inner().tail().is_none() {
                println!(
                    "{}",
                    "run program for longer time so checkpoints have tail"
                        .yellow()
                        .bold()
                );
            }
        }
        Err(err) => {
            println!(
                "{}",
                format!("\n unable to access checkpoints: {:?}", err)
                    .bold()
                    .red()
            );
            println!("{}", "try re-running program for longer time to ensure at least one checkpoint was generated".bold().red());
        }
    }
}

fn shutdown_instance(
    instance: AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
    runtime: Arc<Runtime>,
) {
    println!(
        "{}",
        "waiting for finishing processing remaining blocks...".yellow()
    );
    println!("{}", "press Ctrl+C again to force kill".on_bright_yellow());

    runtime.block_on(async {
        let cloned_for_shutdown = instance.partially_clone_for_forced_shutdown();

        tokio::select! {
            shutdown_join_handle = instance.gracefully_shutdown() => {
                match shutdown_join_handle.unwrap().shutdown_errors.into_option() {
                    None => println!("{}", "attestation blocks builder shut down sucessfully".green()),
                    Some(shutdown_errors) => {
                        if let Some(err) = shutdown_errors.block_listener {
                            println!("{}", format!("failure while waiting for block listener task to terminate, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.resiliency_queue {
                            println!("{}", format!("failure while waiting for resiliency event loop to leave, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.check_connectivity {
                            println!("{}", format!("failure while waiting for check connectivity event loop to leave, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.build_chain {
                            println!("{}", format!("failure while waiting for build chain task to terminate, error: {:?}", err).bright_red());
                        }
                    }
                }
            },
            _ = signal::ctrl_c() => {
                cloned_for_shutdown.force_shutdown().unwrap();
            }
        }
    });
}

#[allow(dead_code)]
fn shutdown_both_instances(
    attestation_chain_builder: AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
    historical_blocks_crawler: AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
    runtime: Arc<Runtime>,
) {
    println!(
        "{}",
        "waiting for finishing processing remaining blocks...".yellow()
    );
    println!("{}", "press Ctrl+C again to force kill".on_bright_yellow());

    runtime.block_on(async {
        let cloned_for_shutdown1 = attestation_chain_builder.partially_clone_for_forced_shutdown();
        let cloned_for_shutdown2 = historical_blocks_crawler.partially_clone_for_forced_shutdown();

        let fut1 = attestation_chain_builder.gracefully_shutdown();
        let fut2 = historical_blocks_crawler.gracefully_shutdown();

        let combined_shutdown_fut = futures::future::try_join(fut1, fut2);

        tokio::select! {
            res = combined_shutdown_fut => {
                let (instance1, instance2) = res.unwrap();
                match instance1.shutdown_errors.into_option() {
                    None => println!("{}", "attestation blocks builder shut down sucessfully".green()),
                    Some(shutdown_errors) => {
                        if let Some(err) = shutdown_errors.block_listener {
                            println!("{}", format!("failure while waiting for block listener task to terminate, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.resiliency_queue {
                            println!("{}", format!("failure while waiting for resiliency event loop to leave, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.check_connectivity {
                            println!("{}", format!("failure while waiting for check connectivity event loop to leave, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.build_chain {
                            println!("{}", format!("failure while waiting for build chain task to terminate, error: {:?}", err).bright_red());
                        }
                    }
                }
                match instance2.shutdown_errors.into_option() {
                    None => println!("{}", "historical blocks crawler shut down sucessfully".green()),
                    Some(shutdown_errors) => {
                        if let Some(err) = shutdown_errors.block_listener {
                            println!("{}", format!("failure while waiting for block listener task to terminate, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.resiliency_queue {
                            println!("{}", format!("failure while waiting for resiliency event loop to leave, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.check_connectivity {
                            println!("{}", format!("failure while waiting for check connectivity event loop to leave, error: {:?}", err).bright_red());
                        }
                        if let Some(err) = shutdown_errors.build_chain {
                            println!("{}", format!("failure while waiting for build chain task to terminate, error: {:?}", err).bright_red());
                        }
                    }
                }
            },
            _ = signal::ctrl_c() => {
                cloned_for_shutdown1.force_shutdown().unwrap();
                cloned_for_shutdown2.force_shutdown().unwrap();
            }
        }
    });
}

// #[allow(dead_code)]
// fn historical_crawler_only_main() {
//     use tokio::sync::mpsc::channel;
//     use attestation_chain::block::Block;
//     use attestation_chain::attestation_fragment::AttestationFragment;
//     use attestation_chain::attestation_checkpoints::AttestationInterval;

//     let args = Args::parse();

//     let config_file_name = match args.config_file {
//         Some(config_file) => {
//             println!("configuration file: {config_file}");
//             config_file
//         },
//         None => {
//             println!("configuration file not specified, defaulted to: {DEFAULT_CONFIG_FILE}");
//             DEFAULT_CONFIG_FILE.to_owned()
//         },
//     };

//     let config_file = std::fs::File::open(config_file_name).unwrap();

//     let config = serde_json::from_reader::<_, Config>(config_file)
//         .unwrap();
//     let wss_url = config.source_chain_wss_server_url;
//     let source_chain_api_server_url = config.source_chain_api_server_url;
//     let block_cache_dir = config.block_cache_dir;
//     let max_num_of_blocks_to_retrieve = config.max_num_of_blocks_to_retrieve.unwrap_or(DEFAULT_MAX_BLOCKS_TO_RETRIEVE);

//     if let Some(cache_dir) = block_cache_dir.as_ref() {
//         println!("{}", format!("NOTE: cache mode is on, blocks will be saved to {cache_dir}").yellow().bold());
//         if let Ok(cache_dir_size) = fs_extra::dir::get_size(cache_dir) {
//             println!("{}", format!("block cache size: {cache_dir_size}").yellow().bold());
//         }
//     }

//     let core_ids = core_affinity::get_core_ids().unwrap();

//     println!("cpu cores: {}", core_ids.len());

//     let runtime = tokio::runtime::Builder::new_multi_thread()
//         .enable_io()
//         .enable_time()
//         .worker_threads(core_ids.len())
//         .on_thread_start(|| {
//             println!("{}", "tokio thread started".bold().blue());
//         })
//         .build()
//         .unwrap();

//     println!("{}", "press Ctrl+C to quit".bold());
//     println!("{}\n{}",
//         format!("backpressure parameter (max number of blocks to remotely retrieve in parallel): {}",
//                 max_num_of_blocks_to_retrieve
//         ).bold(),
//         format!("the higher this parameter is, more chances to keep up with block production rate and higher probability of getting DoS errors",
//         ).bold(),
//     );

//     let runtime = Arc::new(runtime);

//     let checkpoints = Arc::new(RwLock::new(
//         AttestationCheckpointsForDev::with_filename("../data/execution-chain", "checkpoints.json")
//     ));

//     let cancel_on_fatal_failure = CancellationToken::new();
//     let cancel_on_fragment_manager_dead_cloned = cancel_on_fatal_failure.clone();

// //    let historical_stream_start_block = 18000000;
//     let historical_stream_start_block = 19543675 + 1;
//     print_with_timestamp(
//         format!(
//             "historical block crawler will start from block {}",
//             historical_stream_start_block
//         )
//         .bold().cyan()
//     );

//     let (crawler_kickoff_fragment_tx, crawler_kickoff_fragment_rx) = channel::<AttestationInterval>(1);
//     let dummy_interval = AttestationInterval::interval_for(historical_stream_start_block).unwrap();

//     runtime.block_on(crawler_kickoff_fragment_tx.send(dummy_interval));

//     let res = create_historical_blocks_crawler_instance(
//             &source_chain_api_server_url,
//             block_cache_dir.as_deref(),
//             Arc::clone(&runtime),
//             max_num_of_blocks_to_retrieve,
//             cancel_on_fatal_failure.clone(),
//             crawler_kickoff_fragment_rx,
//     );

//     match res {
//         Ok((historical_blocks_crawler, historical_blocks_injector, block_receiver)) => {
//             let historical_blocks_db_manager = create_historical_blocks_fragment_manager_instance(
//                 Arc::clone(&runtime),
//                 Arc::clone(&checkpoints),
//                 block_receiver,
//                 historical_blocks_injector,
//                 cancel_on_fatal_failure.clone()
//             );

//             runtime.block_on(async {
//                 tokio::select! {
//                     _ = signal::ctrl_c() => (),
//                     _ = cancel_on_fragment_manager_dead_cloned.cancelled() => (),
//                 }
//             });

//             shutdown_instance(
//                 historical_blocks_crawler,
//                 Arc::clone(&runtime)
//             );

//             println!("{}", "main task is waiting for historical blocks fragment manager to exit...".yellow());
//             let _ = runtime.block_on(historical_blocks_db_manager.shutdown());

//         },
//         Err(InstanceCreationError::Cancelled) => {
//         },
//         Err(err) => {
//             println!("{}", format!("historical blocks crawler could not start: {:?}", err).red());
//             println!("{}", "db will be built only from new blocks".red());

//         }
//     }

//     println!("{}", "main task is exitting".yellow());
// }
