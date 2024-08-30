use attestation_chain::attestation_checkpoints::AttestationInterval;
use attestation_chain::block::Block;
use attestation_db::json_db::AttestationJsonDB;
use attestation_db::{AttestationDB, AttestationDbError};
use colored::Colorize;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{channel, Receiver, UnboundedReceiver};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::db_manager::{DbManager, DbManagerBuilder};
use crate::print_with_timestamp;
use attestation_chain::AttestationChainParams;
//use crate::OnBlockStopConditionPredicate;

#[allow(dead_code)]
pub(crate) fn create_db_manager_instance(
    //    pub(crate) fn create_db_manager_instance<P: OnBlockStopConditionPredicate>(
    attestation_chain_params: Arc<AttestationChainParams>,
    runtime: Arc<Runtime>,
    db: Arc<RwLock<AttestationJsonDB>>,
    db_block_receiver: UnboundedReceiver<Block>,
) -> (DbManager, Receiver<AttestationInterval>) {
    //    let (crawler_kickoff_fragment_ready_tx, crawler_kickoff_fragment_ready_rx) = channel::<Box<AttestationFragment>>(1);
    let db_cloned = Arc::clone(&db);
    let (crawler_kickoff_block_tx, crawler_kickoff_block_rx) =
        channel::<AttestationInterval>(1);
    let cancellation_token = CancellationToken::new();

    let instance = DbManagerBuilder::new(
//        let instance = DbManagerBuilder::<P>::new(
        Arc::clone(&runtime),
        Arc::clone(&db),
        db_block_receiver,
        cancellation_token,
    )
    .on_block_append_outcome(move |outcome| {
        let mut crawler_kickoff_block_tx = Some(crawler_kickoff_block_tx.clone());
        let attestation_chain_params = Arc::clone(&attestation_chain_params);

        Box::pin(async move {
            match outcome {
                Ok(block) => {
                    let block_number = block.n();
                    print_with_timestamp(
                        format!(
                            "𓈜 block {block_number} appended to attestation fragment", 
                        ).bold().bright_magenta()
                    );
                    if let Some(crawler_kickoff_block_tx) = crawler_kickoff_block_tx.take() {
                        if let Some(prev_interval) = attestation_chain_params.interval_for(block_number) {
                            if let Ok(()) = crawler_kickoff_block_tx.send(prev_interval).await {
    //                            println!("{}", format!("historical blocks crawler can kickoff from fragment {prev_interval:?}").bold().bright_green())
                            }
                        } else {
                            println!("{}", 
                                format!("can't create reference interval for block {block_number}\ncrawler will not start").red()
                            )
                        }
                    }
                },
                Err(AttestationDbError::MisalignedBlockDiscarded(block)) => {
                    print_with_timestamp(format!("DB: block {} is misaligned, discarded", block.n()).yellow());    
                },
                Err(err) => {
                    print_with_timestamp(format!("DB: error on appending block: {err:?}").red());    
                },
            }
        })
    })
    .on_full_fragment_set_outcome(move |result| {
        let db = Arc::clone(&db_cloned);
//        let mut crawler_kickoff_fragment_ready_tx = Some(crawler_kickoff_fragment_ready_tx.clone());

        Box::pin(async move {
//            let crawler_kickoff_fragment_ready_tx = crawler_kickoff_fragment_ready_tx.take();

            match result {
                Ok(fragment_boxed) => {
                    let db = db.read().await;
                    let head = fragment_boxed.head().expect("full fragment has head");
                    let tail = fragment_boxed.tail().expect("full fragment has tail");
                    print_with_timestamp(
                        format!("𓈜𓈜𓈜 fragment {} => ({}, {}) set in DB, fragments in db: {}", 
                            db.key_for(head.n()).expect("fragment was set in db"),
                            tail.n(),
                            head.n(),
                            db.len(),
                        ).bold().bright_green()
                    );
                },
                Err(err) => {
                    print_with_timestamp(
                        format!("failed to set fragment to DB: {err:?}").bright_red()
                    );
                },
            }
        })
    })
    .on_db_manager_task_exitted(move |outcome| {
        Box::pin(async move {
            println!("{}", format!("db manager task exitted, outcome: {outcome:?}").yellow());
        })
    })
    .build_block_listener_db_manager();

    (instance, crawler_kickoff_block_rx)
}
