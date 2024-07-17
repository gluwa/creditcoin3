// use std::sync::Arc;
// use colored::Colorize;
// use tokio::sync::RwLock;
// use tokio::sync::mpsc::{UnboundedReceiver};
// use tokio::runtime::Runtime;
// use tokio_util::sync::CancellationToken;
// use attestation_chain::block::Block;
// use attestation_blocks_online_builder::{BackwardNextHistoricalBlockInjector, BackwardHistoricalBlocksProviderError};
// use attestation_chain::attestation_fragment::{AttestationFragment, AttestationFragmentError};
// use attestation_chain::attestation_checkpoints::{AttestationCheckpoint, AttestationInterval};
// use attestation_chain::attestation_checkpoints_for_dev::{AttestationCheckpointsForDev};
// use attestation_chain::attestation_checkpoints::AttestationCheckpointError;
// use crate::fragment_manager::{FragmentManager, FragmentManagerBuilder};
// use crate::print_with_timestamp;

// pub(crate) fn create_historical_blocks_fragment_manager_instance(
//     runtime: Arc<Runtime>,
//     checkpoints: Arc<RwLock<AttestationCheckpointsForDev>>,
//     block_receiver: UnboundedReceiver<Block>,
//     historical_blocks_injector: BackwardNextHistoricalBlockInjector,
//     cancel_on_fatal_failure: CancellationToken,
// ) -> FragmentManager {
//     let historical_blocks_injector_cloned = historical_blocks_injector.clone();
//     let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();

//     let instance = FragmentManagerBuilder::new(
//         Arc::clone(&runtime),
//         block_receiver,
//     )
//     .on_block_append_outcome(move |outcome| {
//         let historical_blocks_injector = historical_blocks_injector.clone();
//         let cancel_on_fatal_failure = cancel_on_fatal_failure_cloned.clone();

//         Box::pin(async move {
//             match outcome {
//                 Ok(block) => {
//                     print_with_timestamp(
//                         format!(
//                             "𓈜 block {} appended to attestation fragment",
//                             block.n(),
//                         ).bold().bright_magenta().into()
//                     );
//                     match historical_blocks_injector.on_block_appended(&block) {
//                         Ok(_) => (),
//                         Err(err) => {
//                             print_with_timestamp(
//                                 format!("fatal: historical block injector error: {err:?}").bold().red().into()
//                             );
//                             cancel_on_fatal_failure.cancel();
//                         },
//                     }
//                 },
//                 Err(AttestationFragmentError::MisalignedBlock(block)) => {
//                     print_with_timestamp(format!("block {} is misaligned, discarded", block.n()).yellow());
//                 },
//                 Err(err) => {
//                     print_with_timestamp(format!("error on appending block: {err:?}").red());
//                 },
//             }
//         })
//     })
//     .on_checkpoint_ready(move |cp| {
//         let checkpoints = Arc::clone(&checkpoints);
//         let historical_blocks_injector = historical_blocks_injector_cloned.clone();
//         let cancel_on_fatal_failure = cancel_on_fatal_failure.clone();

//         Box::pin(async move {
//             try_prepend_checkpoint(checkpoints, cp, cancel_on_fatal_failure).await;

//             let interval = AttestationInterval::interval_for(cp.n()).expect("interval exists for aligned checkpoint");
//             match historical_blocks_injector.on_fragment_set(interval) {
//                 Ok(_) => (),
//                 Err(BackwardHistoricalBlocksProviderError::GenesisReached) => {
//                     print_with_timestamp(
//                         format!("GENESIS REACHED !!!!!!!!!").bold().bright_green().into()
//                     );
//                 },
//                 Err(err) => {
//                     print_with_timestamp(
//                         format!("historical block injector error: {err:?}").bold().red().into()
//                     );
//                 },
//             }
//         })
//     })
//     .on_task_exitted(move |outcome| {
//         Box::pin(async move {
//             println!("{}", format!("db manager task for historical blocks exitted, outcome: {outcome:?}").yellow());
//         })
//     })
//     .build_historical_blocks_crawler_fragment_manager();

//     instance
// }

// async fn try_prepend_checkpoint(
//     checkpoints: Arc<RwLock<AttestationCheckpointsForDev>>,
//     cp: AttestationCheckpoint,
//     cancel_on_fatal_failure: CancellationToken,
// ) {
//     let n = cp.n();
//     let result = checkpoints.write().await.try_prepend(cp);
//     match &result {
//         Ok(()) => {
//             print_with_timestamp(
//                 format!(
//                     "checkpoint {n} prepended to attestation chain"
//                 ).bold().bright_cyan().into()
//             );
//         },
//         Err(err) => {
//             print_with_timestamp(
//                 format!(
//                     "fatal: failure on prepending checkpoint {n}: {err:?}",
//                 ).red().into()
//             );
//             cancel_on_fatal_failure.cancel();
//         }
//     }
// }
