use crate::create_attestation_block_task::{
    create_attestation_block_task, CreateAttestationBlockError,
};
use crate::network_failures_resilience::ContinuityHandle;
use crate::{AsyncCallbackWithArg, SourceChainBlockIdentifier};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::*;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use ethereum_types::U256;

#[derive(PartialEq, Clone, Debug)]
pub enum Outcome {
    Cancelled,
    Forced,
}

pub(crate) async fn build_attestation_chain_task(
    source_chain_api_server_url: Arc<str>,
    cache_dir: Option<Arc<str>>,
    runtime: Arc<tokio::runtime::Runtime>,
    mut rx: UnboundedReceiver<SourceChainBlockIdentifier>, // receiver from purgatory
    resiliency_sender: UnboundedSender<ContinuityHandle>, // sender to attestation block priority queue
    disconnected: Arc<AtomicBool>, // toggled when network disconnection is discovered
    force_shutdown_token: CancellationToken,
    ongoing_create_attestation_block_tasks: Arc<
        RwLock<HashMap<U256, JoinHandle<Result<U256, CreateAttestationBlockError>>>>,
    >,
    // hook callbacks
    on_announced_block_is_being_processed: Option<AsyncCallbackWithArg<U256, ()>>,
    on_create_attestation_block_outcome: Option<
        AsyncCallbackWithArg<Result<U256, CreateAttestationBlockError>, ()>,
    >,
    on_waiting_to_finish_creating_block_task: Option<AsyncCallbackWithArg<U256, ()>>,
    on_attestation_chain_build_task_exitted: Option<AsyncCallbackWithArg<Outcome, ()>>,
    on_retry_retrieve_block: Option<AsyncCallbackWithArg<(U256, String, u64), ()>>,
    on_toggle_connection_mode: Option<AsyncCallbackWithArg<bool, ()>>,
) {
    let ongoing_create_attestation_block_tasks_cloned =
        Arc::clone(&ongoing_create_attestation_block_tasks);

    let runtime = Arc::clone(&runtime);

    let mut outcome = loop {
        match rx.recv().await {
            Some(block) => {
                let block_number = block.block_number();

                if let Some(ref callback) = on_announced_block_is_being_processed {
                    callback(block_number).await;
                }

                let resiliency_sender = resiliency_sender.clone();
                let ongoing_create_attestation_block_tasks_cloned =
                    Arc::clone(&ongoing_create_attestation_block_tasks_cloned);
                let disconnected = Arc::clone(&disconnected);
                // share the resiliency_queue_event_loop_cancellation_token with create_attestation_block task
                // as they are cancelled at the same time scope
                let create_attestation_block_cancellation_token = force_shutdown_token.clone();

                let on_create_attestation_block_outcome =
                    on_create_attestation_block_outcome.clone();
                let on_retry_retrieve_block = on_retry_retrieve_block.clone();
                let on_toggle_connection_mode = on_toggle_connection_mode.clone();
                let source_chain_api_server_url = Arc::clone(&source_chain_api_server_url);
                let cache_dir = cache_dir.clone();
                // slowly download historical transactions and receipts
                // compute Pedersen hashes and create attestation block
                let task_join_handle = runtime.spawn(async move {
                    let result = create_attestation_block_task(
                        source_chain_api_server_url,
                        cache_dir,
                        block_number,
                        create_attestation_block_cancellation_token,
                        disconnected,
                        500, // retry timeout millis
                        3,   // retrial_attempts,
                        on_retry_retrieve_block,
                        on_toggle_connection_mode,
                    )
                    .await
                    .and_then(|roots| {
                        // at this point the attestation block is crafted, however
                        // before being appended to the attestation chain it is sent
                        // to the resiliency queue where it will wait until all the previous
                        // blocks are also ready
                        resiliency_sender
                            .send(ContinuityHandle::new(block_number, roots))
                            .map_err(|err| {
                                CreateAttestationBlockError::ResiliencyEventLoopDropped(
                                    err.0.block_number(),
                                )
                            })?;

                        Ok(block_number)
                    });
                    // when done, remove this task join handle from the ongoing task hashtable
                    ongoing_create_attestation_block_tasks_cloned
                        .write()
                        .await
                        .remove(&block_number);
                    // invoke callback on successful or failed block creation
                    if let Some(ref callback) = on_create_attestation_block_outcome {
                        callback(result.clone()).await;
                    }
                    result
                });
                // insert task join handle to ongoing task hashtable
                ongoing_create_attestation_block_tasks
                    .write()
                    .await
                    .insert(block_number, task_join_handle);
            }
            None => {
                break Outcome::Cancelled;
            }
        }
    };
    // gracefull shutdown sequence
    if outcome == Outcome::Cancelled {
        let remaining_tasks_join_handle = tokio::spawn(async move {
            // need to copy handles to release the lock early and prevent deadlock
            let handles = ongoing_create_attestation_block_tasks
                .write()
                .await
                .drain()
                .collect::<Vec<_>>();

            for (block_number, handle) in handles.into_iter() {
                if let Some(ref callback) = on_waiting_to_finish_creating_block_task {
                    callback(block_number).await;
                }
                if let Ok(result) = handle.await {
                    if let Some(ref callback) = on_create_attestation_block_outcome {
                        callback(result).await;
                    }
                }
            }
        });

        if !disconnected.load(std::sync::atomic::Ordering::Relaxed) {
            tokio::select! {
                _ = force_shutdown_token.cancelled() => { outcome = Outcome::Forced; },
                _ = remaining_tasks_join_handle => {}
            }
        }
    }

    if let Some(callback) = on_attestation_chain_build_task_exitted {
        callback(outcome).await;
    }
}
