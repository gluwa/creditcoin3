
mod cairo_verify_claim;
mod db_manager;
mod db_manager_instance;
mod historical_blocks_crawler_instance;
mod historical_blocks_db_manager_instance;
mod claim_streams;

use crate::cairo_verify_claim::cairo_verify_claim;
use crate::historical_blocks_crawler_instance::*;
use crate::historical_blocks_db_manager_instance::*;
use attestation_blocks_online_builder::{
    AttestationChainOnlineBuilder, DEFAULT_MAX_BLOCKS_TO_RETRIEVE, SOURCE_BLOCK_TIME_MILLIS,
};
use crate::claim_streams::FromJsonClaimGenerationStream;
//use crate::claim_streams::{SeqClaimGenerationStream, RandomClaimGenerationStream, FromJsonClaimGenerationStream};
use std::pin::Pin;
use attestation_chain::attestation_checkpoints::AttestationInterval;
use attestation_chain::attestation_checkpoints_for_dev::AttestationCheckpointsForDev;
use attestation_db::json_db::AttestationJsonDB;
use attestation_db::AttestationDB;
use clap::Parser;
use colored::Colorize;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::signal;
use tokio::sync::mpsc::channel;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use futures::StreamExt;
use futures_util::stream::Stream;
use anyhow::anyhow;
use ethereum_types::U256;
use either::Either;
use proof::types::{CairoVerifierOutput, StoneProof};
use utils::json_serializable::JsonSerializable;
use prover_primitives::claim::ClaimSerializable;
use poc_config::PocConfig;

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

#[derive(Clone, Debug)]
enum ClaimStreamType {
    Random,
    Sequencial,
    Json,
}

impl From<&str> for ClaimStreamType {
    fn from(s: &str) -> Self {
        match s {
            "random" => Self::Random,
            "seq" => Self::Sequencial,
            "json" => Self::Json,
            _ => panic!("one of following is supported: 'random', 'seq', 'json'"),
        }
    }
}

impl Default for ClaimStreamType {
    fn default() -> Self {
        Self::Json
    }
}

#[derive(Parser, Debug)]
#[command()]
struct Args {
    #[arg(long)]
    config_file: Option<String>,

    #[arg(long)]
    reset_db: bool,

    #[arg(long)]
    claim_stream_type: Option<ClaimStreamType>,

    #[arg(long)]
    dont_stop_on_failure: bool,

    #[arg(long)]
    generate_stone_proof: bool,

    #[arg(long)]
    force_stone_proving: bool,
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

    let prover_config = poc_config.demo_prover().cloned().unwrap_or_else(|| {
        println!("prover config section not present, setting default values");
        Default::default()
    });
    let source_chain_api_server_url = poc_config.source_chain_api_server_url();
    println!("source chain API server url: {source_chain_api_server_url}");
    let block_cache_dir = poc_config.block_cache_url();
    println!("block cache: {block_cache_dir:?}");
    let checkpoints_path = match poc_config.execution_chain_url() {
        Some(checkpoints_path) => checkpoints_path.to_owned(),
        None => {
            println!("execution chain url, setting default value");
            PocConfig::default_execution_chain_url()
        }
    };
    println!("checkpoints at: {checkpoints_path}");
    let db_url = prover_config
        .db_url()
        .expect("at worst case was set to default");
    println!("db: {db_url}");
    let mut db = AttestationJsonDB::try_create(db_url).unwrap();

    if args.reset_db {
        match db.reset() {
            Ok(()) => println!("{}", "db has been reset".to_owned().bold().yellow()),
            Err(err) => {
                println!("{}", format!("db reset failure: {err:?}",).red());
                return;
            }
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

    let runtime = Arc::new(runtime);
    let runtime_cloned = Arc::clone(&runtime);
    let db = Arc::new(RwLock::new(db));
    let _source_chain_api_server_url_cloned = source_chain_api_server_url.to_owned();

    let checkpoints = AttestationCheckpointsForDev::with_execution_chain_url(&checkpoints_path);
    println!("accessing checkpoints at {}", checkpoints.full_path());

    let claim_listener_join_handle = runtime.spawn(async move {

        let claim_stream_type = args.claim_stream_type.unwrap_or_default();
        println!("{}", format!("claim stream: {claim_stream_type:?}").bold());

        let mut claim_stream: Pin<Box<dyn Stream<Item=ClaimSerializable> + Send + Sync>> = match claim_stream_type {
            ClaimStreamType::Sequencial => unimplemented!("uncomment and fix SeqClaimGenerationStream"),
                // Box::pin(
                //     SeqClaimGenerationStream::new(
                //         checkpoints.clone(), 
                //         poc_config.block_cache_url(), 
                //     )
                // ),
            ClaimStreamType::Random => unimplemented!("uncomment and fix RandomClaimGenerationStream"),
                // Box::pin(
                //     RandomClaimGenerationStream::new(
                //         checkpoints.clone(), 
                //         poc_config.block_cache_url(), 
                //     )
                // ),
            ClaimStreamType::Json => {
                let claims_json_file = "../data/claims.json";
                println!("{}", format!("claim stream will be loaded from {claims_json_file}").bold());
                Box::pin(
                    match FromJsonClaimGenerationStream::try_create(claims_json_file) {
                        Ok(claim_stream) => claim_stream,
                        Err(err) => {
                            println!("{}", format!("unable to fetch claims from file {claims_json_file}: {err:?}",).red());
                            return;
                        }
                    }
                )
            },
        };

        while let Some(claim) = claim_stream.next().await {
            let res = build_db_and_submit_claim(
                poc_config.clone(), 
                Arc::clone(&runtime_cloned), 
                Arc::clone(&db), 
                checkpoints.clone(), 
                claim.clone(),
                args.generate_stone_proof,
                args.force_stone_proving,
            )
            .await;

            match res {
                Ok(Either::Left(stone_proof)) => {
                    let fname = "../data/node-side-proofs/proof.json";
                    //let proof = stone_proof.proof();
                    //let public_input = StoneProofPublicInput::try_from(proof);

                    println!("saving public input to {fname}");

                    stone_proof
                        // .strip_off_annotations()
                        // .strip_off_prover_config()
                        // .strip_off_private_input()
                        .to_file(fname).unwrap();
                },
        
                Ok(Either::Right(_output)) => {
                    // if output.claim_id.kind == ClaimKind::Tx {
                    //     let txs = TypedTransaction::fetch_all(
                    //         &source_chain_api_server_url_cloned, 
                    //         None, 
                    //         claim.id().block_item_id.block_number()
                    //     )
                    //     .await
                    //     .unwrap();

                    //     println!("testing cairo generated output locally to match claim values...");
                    //     let tx = &txs[claim.id().block_item_id.index() as usize];
                    //     let payload_bytes = &tx.payload_bytes()[..];
                    //     let rlp = rlp::Rlp::new(payload_bytes);
                    //     let query = create_sample_query(&tx);
                    //     let claim = common::claim::Claim::try_create(claim.id().clone(), query.clone(), rlp).unwrap();
                        
                    //     match claim.validate_fields(&output.claim_fields, &output.query_hash) {
                    //         Ok(_) => {
                    //             println!("{}", format!("claim validated").green().bold());
                    //         },
                    //         Err(err) => {
                    //             println!("{}", format!("error: {:?}", err).red());
                    //         }
                    //     }   
                    // }          
                },
        
                Err(err) => {
                    println!("{}", format!("error: {:?}", err).red());
                    if !args.dont_stop_on_failure {
                        break;
                    }
                }
            }            
        }
    });

    runtime.block_on(async {
        tokio::select! {
            _ = signal::ctrl_c() => (),
            _ = claim_listener_join_handle => (),
        }
    });
    println!("main task is exitting");
}

async fn build_db_and_submit_claim(
    poc_config: PocConfig,
    runtime: Arc<Runtime>,
    db: Arc<RwLock<AttestationJsonDB>>,
    mut checkpoints: AttestationCheckpointsForDev,
    claim: ClaimSerializable,
    generate_stone_proof: bool,
    force_stone_proving: bool,
) -> anyhow::Result<Either<StoneProof, CairoVerifierOutput>> {
//) -> anyhow::Result<Option<StoneProof>> {

    let source_chain_api_server_url = poc_config.source_chain_api_server_url();
    let block_cache_dir = poc_config.block_cache_url();

    checkpoints
        .poll()
        .map_err(|err| anyhow!("unable to access checkpoints: {err:?}\nrun attestor simulator to generate one or some checkpoints"))?;
    let checkpoints_tail = checkpoints
                            .inner()
                            .tail()
                            .ok_or(anyhow!("inconsistent checkpoints: no tail. Try running attestor simulator for longer time period"))?;
    let checkpoints_stabilized_head = checkpoints
                                        .inner()
                                        .stabilized_head()
                                        .ok_or(anyhow!("inconsistent checkpoints: no stabilized head. Try running attestor simulator for longer time period"))?;
    let checkpoints_head = checkpoints
                            .inner()
                            .head()
                            .ok_or(anyhow!("inconsistent checkpoints: no head. Try running attestor simulator for longer time period"))?;
    println!(
        "{}",
        format!(
            "checkpoints [tail@{} <---...--> stabilized head@{} <--> recent head@{}]",
            checkpoints_tail, checkpoints_stabilized_head, checkpoints_head
        )
        .bold()
    );

    println!("{}", format!("{claim:?}").bold().blue());
    let claim_block_number = claim.id().block_item_id.block_number();
    let claim_checkpoint = checkpoints
                                .inner()
                                .checkpoint_for(claim_block_number)
                                .map(|cp| cp.n())
                                .ok_or(anyhow!("claim block number {} matches no checkpoints", claim_block_number))?;  

    println!("{}", format!("claim checkpoint: {claim_checkpoint}").bold().blue());
    let claim_attestation_fragment = db.read().await.get_fragment_for(claim_block_number);

    let claim_attestation_fragment = match claim_attestation_fragment {
        Some(claim_attestation_fragment) => {
            println!("attestation fragment for claim localized in db");
            claim_attestation_fragment
        }
        None => {
            println!("{}",
                "attestation fragment for claim not present in db, will try to fetch historical blocks from source chain and update db".yellow()
            );

            // let att_interval = AttestationInterval::interval_for(claim_block_number).unwrap();
            // let prev_checkpoint = checkpoints.inner().checkpoint_for(att_interval.tail()).unwrap();
            // db.write().await.try_append_block(attestation_chain::block::Block::from_block_number_and_digest(
            //     prev_checkpoint.n(),
            //     *prev_checkpoint.digest(),
            // ))
            // .unwrap();

            let start_interval = AttestationInterval::interval_for(checkpoints_tail)
                .expect("interval exists for aligned checkpoint");
            let crawler_kickoff_block = skip_existing_fragments(
                                            Arc::clone(&db), 
                                            start_interval, 
                                            claim_checkpoint
                                        )
                                        .await?;
            let stop_condition = StopCondition::OnBlockReached(
                Arc::new(move |curr_block| curr_block >= claim_checkpoint)
            );
            build_attestation_db(
                source_chain_api_server_url,
                block_cache_dir,
                Arc::clone(&runtime),
                Arc::clone(&db),
                crawler_kickoff_block,
                stop_condition,
            ).
            await?;
            
            db.read()
                .await
                .get_fragment_for(claim_block_number)
                .ok_or(anyhow!(
                        format!(
                            "something went wrong - fragment for {} is not in db - should be explored",
                            claim_block_number
                        )
                ))?
        }        
    };
    cairo_verify_claim(
        source_chain_api_server_url,
        claim,
        &claim_attestation_fragment,
        checkpoints.inner(),
        generate_stone_proof,
        force_stone_proving
    )
    .await
    .map_err(|err| anyhow!("claim verification failure: {err:?}"))
}

async fn skip_existing_fragments(
    db: Arc<RwLock<AttestationJsonDB>>,
    start_interval: AttestationInterval,
    claim_checkpoint: U256,
) -> anyhow::Result<U256> {
    let mut next_interval = start_interval;
    let mut crawler_kickoff_block = next_interval.tail();
    while db.read().await.fragment_exists(&next_interval) 
        && 
        claim_checkpoint != next_interval.head() {

        print_with_timestamp(
            format!(
                "fragment {} -> {:?} already set in DB, skipping",
                <AttestationJsonDB as AttestationDB>::key_for(next_interval.head())
                    .unwrap(),
                next_interval,
            )
            .yellow(),
        );
        
        next_interval = next_interval.next();
        crawler_kickoff_block = next_interval.tail() + 1;
    }

    let (db_head, db_recent_tail) = {
        let db = db.read().await;
        (db.recent_fragment().head().cloned(), db.recent_fragment().tail().cloned())
    };
    if let Some(db_head) = db_head {
        if crawler_kickoff_block > db_head.n() + 1 {
            return Err(anyhow!(format!(
                "db state is inconsistent with respect of checkpoints state, db head@{}, needs to be @{}\ntry running with --reset-db flag",
                db_head.n(),
                crawler_kickoff_block - 1
            )));
        } else if crawler_kickoff_block >= db_recent_tail.expect("has head ergo has tail").n() {
            crawler_kickoff_block = db_head.n() + 1;
        }
    }
    Ok(crawler_kickoff_block)
}

async fn build_attestation_db(
    source_chain_api_server_url: &str,
    block_cache_dir: Option<&str>,
    runtime: Arc<Runtime>,
    db: Arc<RwLock<AttestationJsonDB>>,
    crawler_kickoff_block: U256,
    stop_condition: StopCondition,
) -> anyhow::Result<()> {
    println!("{}", "press Ctrl+C to quit".bold());

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

    let cancel_on_fatal_failure = CancellationToken::new();
    let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();

    let (crawler_kickoff_fragment_tx, crawler_kickoff_fragment_rx) = channel(1);
    let _ = crawler_kickoff_fragment_tx.send(crawler_kickoff_block).await;

    let (historical_blocks_crawler, db_block_receiver) = create_historical_blocks_crawler_instance(
        source_chain_api_server_url,
        block_cache_dir,
        Arc::clone(&runtime),
        DEFAULT_MAX_BLOCKS_TO_RETRIEVE,
        cancel_on_fatal_failure,
        crawler_kickoff_fragment_rx,
        StopCondition::FatalFailure,
//        stop_condition.clone(),
    )
    .await
    .map_err(|err| anyhow!("{err:?}"))?;

    let historical_blocks_db_manager = create_historical_blocks_db_manager_instance(
        Arc::clone(&runtime),
        Arc::clone(&db),
        db_block_receiver,
        stop_condition,
    );

    let cancelled_by_user = 
        tokio::select! {
            _ = signal::ctrl_c() => true,
            _ = cancel_on_fatal_failure_cloned.cancelled() => false,
        };

    let success = shutdown_instance(historical_blocks_crawler).await;
    let success = success && !cancelled_by_user;

    println!(
        "{}",
        "main task is waiting for historical blocks db manager to exit...".yellow()
    );
    let _ = historical_blocks_db_manager.wait_for_stop_condition().await;
    if success {Ok(())} else {Err(anyhow!("db building task cancelled or shutdown error"))}
//    println!("{}", "main task is exitting".yellow());
//    success
}

async fn shutdown_instance(
    instance: AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
) -> bool {
    println!(
        "{}",
        "waiting for finishing processing remaining blocks...".yellow()
    );
    println!("{}", "press Ctrl+C again to force kill".on_bright_yellow());

    let cloned_for_shutdown = instance.partially_clone_for_forced_shutdown();

    tokio::select! {
        shutdown_join_handle = instance.gracefully_shutdown() => {
            match shutdown_join_handle.unwrap().shutdown_errors.into_option() {
                None => {
                    println!("{}", "attestation blocks builder shut down sucessfully".green());
                    true
                },
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
                    false
                }
            }
        },
        _ = signal::ctrl_c() => {
            cloned_for_shutdown.force_shutdown().unwrap();
            false
        }
    }
}

// pub(crate) fn create_sample_query(tx: &Transaction) -> TxClaimQuery {
// //    use common::claim_query::{Eip1559TxClaimQueryField, LegacyTxClaimQueryField, Eip2930TxClaimQueryField, Eip4844TxClaimQueryField};
//     use prover_primitives::claim_query::{Eip1559TxClaimQueryField, LegacyTxClaimQueryField, Eip4844TxClaimQueryField, Eip2930TxClaimQueryField};
//     use std::collections::HashSet;

//     println!("{}", format!("transaction type: {:?}", tx.tx_type()).bold().cyan());

//     match tx.tx_type() {
//         None => TxClaimQuery::try_from(
//             vec![
//                 LegacyTxClaimQueryField::To,
//                 LegacyTxClaimQueryField::SingleDataRelativeRange(None),
//                 LegacyTxClaimQueryField::Signature,
//             ].into_iter().collect::<HashSet<_>>()
//         ).unwrap(),
//         Some(1) => TxClaimQuery::try_from(
//             vec![
//                 Eip2930TxClaimQueryField::ChainId, 
//                 Eip2930TxClaimQueryField::To,
//                 Eip2930TxClaimQueryField::SingleDataRelativeRange(None),
//                 Eip2930TxClaimQueryField::Signature,
//             ].into_iter().collect::<HashSet<_>>()
//         ).unwrap(),
//         Some(2) => TxClaimQuery::try_from(
//             vec![
//                 Eip1559TxClaimQueryField::ChainId, 
//                 Eip1559TxClaimQueryField::To,
//                 Eip1559TxClaimQueryField::SingleDataRelativeRange(None),
//                 Eip1559TxClaimQueryField::Signature,
//             ].into_iter().collect::<HashSet<_>>()
//         ).unwrap(),
//         Some(3) => TxClaimQuery::try_from(
//             vec![
//                 Eip4844TxClaimQueryField::ChainId, 
//                 Eip4844TxClaimQueryField::To,
//                 Eip4844TxClaimQueryField::SingleDataRelativeRange(None),
//                 Eip4844TxClaimQueryField::Signature,
//             ].into_iter().collect::<HashSet<_>>()
//         ).unwrap(),
//         _ => unimplemented!("tx type not supported"),
//     }
// }

// fn main_main() {
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

//     let db = AttestationJsonDB::try_create("../data/live_db").unwrap();
//     let db = Arc::new(RwLock::new(db));

// //    let checkpoints = Arc::new(RwLock::new(AttestationCheckpoints::new()));

//     let cancel_on_fatal_failure = CancellationToken::new();
//     let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();

//     let (attestation_chain_builder, db_block_receiver) = create_attestation_blocks_builder_instance(
//         &source_chain_api_server_url,
//         &wss_url,
//         block_cache_dir.as_deref(),
//         Arc::clone(&runtime),
//         max_num_of_blocks_to_retrieve,
//         cancel_on_fatal_failure.clone()
//     );

//     let (db_manager, crawler_kickoff_block_rx) = create_db_manager_instance(
//         Arc::clone(&runtime),
//         Arc::clone(&db),
// //        Arc::clone(&checkpoints),
//         db_block_receiver,
//     );

//     let res = create_historical_blocks_crawler_instance(
//             &source_chain_api_server_url,
//             block_cache_dir.as_deref(),
//             Arc::clone(&runtime),
//             max_num_of_blocks_to_retrieve,
//             cancel_on_fatal_failure,
//             crawler_kickoff_block_rx,
//     );

//     match res {
//         Ok((historical_blocks_crawler, historical_blocks_injector, db_block_receiver)) => {
//             let historical_blocks_db_manager = create_historical_blocks_db_manager_instance(
//                 Arc::clone(&runtime),
//                 Arc::clone(&db),
// //                Arc::clone(&checkpoints),
//                 db_block_receiver,
//                 historical_blocks_injector,
//                 StopCondition::default(),
//             );

//             runtime.block_on(async {
//                 tokio::select! {
//                     _ = signal::ctrl_c() => (),
//                     _ = cancel_on_fatal_failure_cloned.cancelled() => (),
//                 }
//             });

//             shutdown_both_instances(
//                 attestation_chain_builder,
//                 historical_blocks_crawler,
//                 Arc::clone(&runtime)
//             );

//             println!("{}", "main task is waiting for historical blocks db manager to exit...".yellow());
//             let _ = runtime.block_on(historical_blocks_db_manager.shutdown());

//         },
//         Err(InstanceCreationError::Cancelled) => {
//             shutdown_instance(attestation_chain_builder, Arc::clone(&runtime));
//         },
//         Err(err) => {
//             println!("{}", format!("historical blocks crawler could not start: {:?}", err).red());
//             println!("{}", "db will be built only from new blocks".red());

//             runtime.block_on(async {
//                 tokio::select! {
//                     _ = signal::ctrl_c() => (),
//                     _ = cancel_on_fatal_failure_cloned.cancelled() => (),
//                 }
//             });

//             shutdown_instance(attestation_chain_builder, Arc::clone(&runtime));
//         }
//     }

//     println!("{}", "main task is waiting for db manager to exit...".yellow());
//     let _ = runtime.block_on(db_manager.shutdown());

//     println!("{}", "main task is exitting".yellow());
// }

// fn shutdown_both_instances(
//     attestation_chain_builder: AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
//     historical_blocks_crawler: AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
//     runtime: Arc<Runtime>,
// ) {
//     println!("{}", "waiting for finishing processing remaining blocks...".yellow());
//     println!("{}", "press Ctrl+C again to force kill".on_bright_yellow());

//     runtime.block_on(async {
//         let cloned_for_shutdown1 = attestation_chain_builder.partially_clone_for_forced_shutdown();
//         let cloned_for_shutdown2 = historical_blocks_crawler.partially_clone_for_forced_shutdown();

//         let fut1 = attestation_chain_builder.gracefully_shutdown();
//         let fut2 = historical_blocks_crawler.gracefully_shutdown();

//         let combined_shutdown_fut = futures::future::try_join(fut1, fut2);

//         tokio::select! {
//             res = combined_shutdown_fut => {
//                 let (instance1, instance2) = res.unwrap();
//                 match instance1.shutdown_errors.into_option() {
//                     None => println!("{}", "attestation blocks builder shut down sucessfully".green()),
//                     Some(shutdown_errors) => {
//                         if let Some(err) = shutdown_errors.block_listener {
//                             println!("{}", format!("failure while waiting for block listener task to terminate, error: {:?}", err).bright_red());
//                         }
//                         if let Some(err) = shutdown_errors.resiliency_queue {
//                             println!("{}", format!("failure while waiting for resiliency event loop to leave, error: {:?}", err).bright_red());
//                         }
//                         if let Some(err) = shutdown_errors.check_connectivity {
//                             println!("{}", format!("failure while waiting for check connectivity event loop to leave, error: {:?}", err).bright_red());
//                         }
//                         if let Some(err) = shutdown_errors.build_chain {
//                             println!("{}", format!("failure while waiting for build chain task to terminate, error: {:?}", err).bright_red());
//                         }
//                     }
//                 }
//                 match instance2.shutdown_errors.into_option() {
//                     None => println!("{}", "historical blocks crawler shut down sucessfully".green()),
//                     Some(shutdown_errors) => {
//                         if let Some(err) = shutdown_errors.block_listener {
//                             println!("{}", format!("failure while waiting for block listener task to terminate, error: {:?}", err).bright_red());
//                         }
//                         if let Some(err) = shutdown_errors.resiliency_queue {
//                             println!("{}", format!("failure while waiting for resiliency event loop to leave, error: {:?}", err).bright_red());
//                         }
//                         if let Some(err) = shutdown_errors.check_connectivity {
//                             println!("{}", format!("failure while waiting for check connectivity event loop to leave, error: {:?}", err).bright_red());
//                         }
//                         if let Some(err) = shutdown_errors.build_chain {
//                             println!("{}", format!("failure while waiting for build chain task to terminate, error: {:?}", err).bright_red());
//                         }
//                     }
//                 }
//             },
//             _ = signal::ctrl_c() => {
//                 cloned_for_shutdown1.force_shutdown().unwrap();
//                 cloned_for_shutdown2.force_shutdown().unwrap();
//             }
//         }
//     });
// }

// #[allow(dead_code)]
// fn historical_crawler_only_main() {
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

//     let db = AttestationJsonDB::try_create("../data/live_db").unwrap();
//     let db = Arc::new(RwLock::new(db));

// //    let checkpoints = Arc::new(RwLock::new(AttestationCheckpoints::new()));

//     let cancel_on_fatal_failure = CancellationToken::new();
//     let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();

// //    let historical_stream_start_block = 18000000;
//     let historical_stream_start_block = 19543675 + 1;
//     print_with_timestamp(
//         format!(
//             "historical block crawler will start from block {}",
//             historical_stream_start_block
//         )
//         .bold().cyan()
//     );

//     let (crawler_kickoff_fragment_tx, crawler_kickoff_fragment_rx) = channel::<u64>(1);
// //    let (crawler_kickoff_fragment_tx, crawler_kickoff_fragment_rx) = channel::<AttestationInterval>(1);
//     let dummy_interval = AttestationInterval::interval_for(historical_stream_start_block).unwrap();
// //    dummy_fragment.try_append_block(Block::new(historical_stream_start_block, 0u64.into(), 0u64.into()));
//     runtime.block_on(crawler_kickoff_fragment_tx.send(dummy_interval));

//     let res = create_historical_blocks_crawler_instance(
//             &source_chain_api_server_url,
//             block_cache_dir.as_deref(),
//             Arc::clone(&runtime),
//             max_num_of_blocks_to_retrieve,
//             cancel_on_fatal_failure,
//             crawler_kickoff_fragment_rx,
//     );

//     match res {
//         Ok((historical_blocks_crawler, historical_blocks_injector, db_block_receiver)) => {
//             let historical_blocks_db_manager = create_historical_blocks_db_manager_instance(
//                 Arc::clone(&runtime),
//                 Arc::clone(&db),
//                 db_block_receiver,
//                 historical_blocks_injector,
//                 StopCondition::default(),
//             );

//             runtime.block_on(async {
//                 tokio::select! {
//                     _ = signal::ctrl_c() => (),
//                     _ = cancel_on_fatal_failure_cloned.cancelled() => (),
//                 }
//             });

//             shutdown_instance(
//                 historical_blocks_crawler,
//                 Arc::clone(&runtime)
//             );

//             println!("{}", "main task is waiting for historical blocks db manager to exit...".yellow());
//             let _ = runtime.block_on(historical_blocks_db_manager.shutdown());

//         },
//         Err(InstanceCreationError::Cancelled) => {
//         },
//         Err(err) => {
//             println!("{}", format!("historical blocks crawler could not start: {:?}", err).red());
//         }
//     }

//     println!("{}", "main task is exitting".yellow());
// }
