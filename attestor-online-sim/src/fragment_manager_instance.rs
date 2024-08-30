use crate::fragment_manager::{FragmentManager, FragmentManagerBuilder};
use attestation_chain::attestation_checkpoints::{AttestationCheckpoint, AttestationInterval};
use attestation_chain::attestation_checkpoints_for_dev::AttestationCheckpointsForDev;
use attestation_chain::attestation_fragment::AttestationFragmentError;
use attestation_chain::block::Block;
use colored::Colorize;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{channel, Receiver, Sender, UnboundedReceiver};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use attestation_chain::AttestationChainParams;
use crate::print_with_timestamp;

pub(crate) fn create_fragment_manager_instance(
    attestation_chain_params: Arc<AttestationChainParams>,
    runtime: Arc<Runtime>,
    checkpoints: Arc<RwLock<AttestationCheckpointsForDev>>,
    block_receiver: UnboundedReceiver<Block>,
    cancel_on_fatal_failure: CancellationToken,
) -> (
    FragmentManager,
    Receiver<AttestationInterval>,
) {
    let checkpoints_cloned = Arc::clone(&checkpoints);
    let (_crawler_kickoff_block_tx, crawler_kickoff_block_rx) =
        channel::<AttestationInterval>(1);
    let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();

    let instance = FragmentManagerBuilder::new(
        Arc::clone(&attestation_chain_params),
        Arc::clone(&runtime),
        block_receiver,
    )
    .on_block_append_outcome(move |outcome| {
        let attestation_chain_params = Arc::clone(&attestation_chain_params);
        let checkpoints = Arc::clone(&checkpoints);
//        let mut crawler_kickoff_block_tx = Some(crawler_kickoff_block_tx.clone());
        let mut crawler_kickoff_block_tx: Option<Sender<AttestationInterval>> = None;
        let cancel_on_fatal_failure = cancel_on_fatal_failure_cloned.clone();

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
                            } else {
                                println!("{}", "error on crawler starting".to_owned().red());
                            }
                        } else {
                            println!("{}", 
                                format!("can't create reference interval for block {block_number}\ncrawler will not start").red()
                            )
                        }
                    }
                    // if block is aligned here this means it's a beginning of the very first fragment
                    // it can't be a valid checkpoint as it doesn't have a predecessor
                    // checkpoint corresponding to this block must be prepended from the previous fragment
                    if !attestation_chain_params.is_aligned(block.n()) {
                        try_append_checkpoint(
                            checkpoints,
                            AttestationCheckpoint::from(&block),
                            cancel_on_fatal_failure
                        )
                        .await;
                    }
                },
                Err(AttestationFragmentError::MisalignedBlock(block)) => {
                    print_with_timestamp(format!("block {} is misaligned, discarded", block.n()).yellow());    
                },
                Err(err) => {
                    print_with_timestamp(format!("error on appending block: {err:?}").red());    
                },
            }
        })
    })
    .on_checkpoint_ready(move |checkpoint| {
        let checkpoints = Arc::clone(&checkpoints_cloned);
        let cancel_on_fatal_failure = cancel_on_fatal_failure.clone();

        Box::pin(async move {
            try_append_checkpoint(checkpoints, checkpoint, cancel_on_fatal_failure).await;
        })
    })
    .on_task_exitted(move |outcome| {
        Box::pin(async move {
            println!("{}", format!("fragment manager task exitted, outcome: {outcome:?}").yellow());
        })
    })
    .build_block_listener_manager();

    (instance, crawler_kickoff_block_rx)
}

async fn try_append_checkpoint(
    checkpoints: Arc<RwLock<AttestationCheckpointsForDev>>,
    cp: AttestationCheckpoint,
    cancel_on_fatal_failure: CancellationToken,
) {
    let n = cp.n();
    let result = checkpoints.write().await.try_append(cp);
    match &result {
        Ok(()) => {
            print_with_timestamp(
                format!("checkpoint {n} appended to attestation chain",)
                    .bold()
                    .bright_cyan(),
            );
        }
        Err(err) => {
            print_with_timestamp(
                format!("fatal: failure on appending checkpoint {n}: {err:?}",).red(),
            );
            cancel_on_fatal_failure.cancel();
        }
    }
}
