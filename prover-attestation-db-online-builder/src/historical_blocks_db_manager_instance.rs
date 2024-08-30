use crate::db_manager::{DbManager, DbManagerBuilder};
use crate::print_with_timestamp;
use crate::StopCondition;
use attestation_chain::attestation_checkpoints::AttestationInterval;
use attestation_chain::block::Block;
use attestation_db::json_db::AttestationJsonDB;
use attestation_db::{AttestationDB, AttestationDbError};
use colored::Colorize;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use attestation_chain::AttestationChainParams;

pub(crate) fn create_historical_blocks_db_manager_instance(
    attestation_chain_params: Arc<AttestationChainParams>,
    runtime: Arc<Runtime>,
    db: Arc<RwLock<AttestationJsonDB>>,
    db_block_receiver: UnboundedReceiver<Block>,
    stop_condition: StopCondition,
) -> DbManager {
    let db_cloned = Arc::clone(&db);
    let attestation_chain_params_cloned = Arc::clone(&attestation_chain_params);
    let cancellation_token = CancellationToken::new();
    let cancellation_token_cloned = cancellation_token.clone();
    let stop_condition_cloned = stop_condition.clone();

    let instance = DbManagerBuilder::new(
        Arc::clone(&runtime),
        Arc::clone(&db),
        db_block_receiver,
        cancellation_token.clone(),
    )
    .on_block_append_outcome(move |outcome| {
        let db = Arc::clone(&db);
        let attestation_chain_params = Arc::clone(&attestation_chain_params_cloned);
        let cancellation_token = cancellation_token_cloned.clone();
        let stop_condition = stop_condition.clone();

        Box::pin(async move {
            match outcome {
                Ok(block) => {
                    print_with_timestamp(
                        format!("𓈜 block {} appended to attestation fragment", block.n(),)
                            .bold()
                            .bright_magenta(),
                    );
                }
                Err(AttestationDbError::MisalignedBlockDiscarded(block)) => {
                    print_with_timestamp(
                        format!("DB: block {} is misaligned, discarded", block.n()).yellow(),
                    );
                }
                Err(AttestationDbError::FragmentAlreadySet(fragment_interval)) => {
                    let _next_interval = skip_existing_fragments(
                        attestation_chain_params,
                        db,
                        fragment_interval.head(),
                        cancellation_token.clone(),
                        stop_condition,
                    )
                    .await;
                }
                Err(err) => {
                    print_with_timestamp(
                        format!("DB: error on appending block: {err:?}, exitting").red(),
                    );
                    cancellation_token.cancel();
                }
            }
        })
    })
    .on_full_fragment_set_outcome(move |result| {
        let db = Arc::clone(&db_cloned);
        let attestation_chain_params = Arc::clone(&attestation_chain_params);
        let cancellation_token = cancellation_token.clone();
        let stop_condition = stop_condition_cloned.clone();

        Box::pin(async move {
            match result {
                Ok(fragment_boxed) => {
                    let head = fragment_boxed.head().unwrap().n();
                    let tail = fragment_boxed.tail().unwrap().n();
                    {
                        let db = db.read().await;
                        print_with_timestamp(
                            format!(
                                "𓈜𓈜𓈜 fragment {} => ({}, {}) set in DB, fragments in db: {}",
                                db.key_for(head).unwrap(),
                                tail,
                                head,
                                db.len(),
                            )
                            .bold()
                            .bright_green(),
                        );
                    }
                    let interval = attestation_chain_params.interval_for(head)
                        .expect("full fragment defines interval");
                    if let StopCondition::OnBlockReached(ref p) = stop_condition {
                        if p(interval.head()) {
                            cancellation_token.cancel();
                        }
                    }
                    let _next_interval = skip_existing_fragments(
                        Arc::clone(&attestation_chain_params),
                        db,
                        interval.next(&attestation_chain_params).head(),
                        cancellation_token.clone(),
                        stop_condition,
                    )
                    .await;
                }
                Err(AttestationDbError::FragmentAlreadySet(fragment_interval)) => {
                    let _next_interval = skip_existing_fragments(
                        attestation_chain_params,
                        db,
                        fragment_interval.head(),
                        cancellation_token.clone(),
                        stop_condition,
                    )
                    .await;
                }
                Err(err) => {
                    print_with_timestamp(
                        format!("failed to set fragment to DB: {err:?}, exitting").bright_red(),
                    );
                    cancellation_token.cancel();
                }
            }
        })
    })
    .on_db_manager_task_exitted(move |outcome| {
        Box::pin(async move {
            println!(
                "{}",
                format!("db manager task for historical blocks exitted, outcome: {outcome:?}")
                    .yellow()
            );
        })
    })
    .build_historical_blocks_crawler_db_manager();

    instance
}

async fn skip_existing_fragments(
    attestation_chain_params: Arc<AttestationChainParams>,
    db: Arc<RwLock<AttestationJsonDB>>,
    fragment_head: u64,
    cancellation_token: CancellationToken,
    stop_condition: StopCondition,
) -> AttestationInterval {
    //                    let tail = fragment_boxed.tail().unwrap().n();
    let mut interval = attestation_chain_params.interval_for(fragment_head)
        .expect("interval exists for aligned checkpoint");
    while db.read().await.fragment_exists(&interval) {
        print_with_timestamp(format!("fragment {interval:?} already set in DB").yellow());
        // if stop_condition == StopCondition::DbFragmentSet(interval) {
        //     cancellation_token.cancel();
        // }
        if let StopCondition::OnBlockReached(ref p) = stop_condition {
            if p(interval.head()) {
                cancellation_token.cancel();
                break;
            }
        }
        interval = interval.next(&attestation_chain_params);
    }
    interval
}
