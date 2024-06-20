#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

use crate::create_attestation_block_task::CreateAttestationBlockError;
use crate::purgatory::BlockPurgatoryQueue;
use crate::{AsyncCallbackWithArg, SourceChainBlockIdentifier, SourceChainBlockStream};
use ethereum_types::U256;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;

#[derive(PartialEq, Clone, Debug)]
pub enum Outcome {
    Cancelled,
    Forced,
    SenderError(SourceChainBlockIdentifier),
    StreamDropped,
    None,
}

pub(crate) async fn ethereum_block_listener<
    const PURGATORY_PERIOD_MILLIS: u128,
    BlockStream: SourceChainBlockStream,
>(
    mut block_stream: Box<BlockStream>,
    append_to_chain_sender: UnboundedSender<SourceChainBlockIdentifier>,
    ongoing_create_attestation_block_tasks: Arc<
        RwLock<HashMap<U256, JoinHandle<Result<U256, CreateAttestationBlockError>>>>,
    >,
    cancellation_token: CancellationToken,
    max_num_of_blocks_to_expulse: usize, // backpressure-related parameter

    on_block_announced_on_source_chain: Option<
        AsyncCallbackWithArg<SourceChainBlockIdentifier, ()>,
    >,
    on_send_block_to_appending_task_outcome: Option<
        AsyncCallbackWithArg<
            Result<SourceChainBlockIdentifier, SendError<SourceChainBlockIdentifier>>,
            (),
        >,
    >,
    on_leaving_block_listener_event_loop: Option<AsyncCallbackWithArg<Outcome, ()>>,
    on_block_listener_event_loop_left: Option<AsyncCallbackWithArg<Outcome, ()>>,
    on_backpressure_applied: Option<AsyncCallbackWithArg<(usize, usize), ()>>,
) {
    let mut purgatory_queue = BlockPurgatoryQueue::<PURGATORY_PERIOD_MILLIS>::new();

    let mut outcome = loop {
        let block_stream = block_stream.deref_mut();

        tokio::select! {
            // wait for next source block announcement
            received = block_stream.next() => {
                match received {
                    Some(block) => {
                        match block.try_into() {
                            Ok(block_identifier) => {
                                if let Some(ref callback) = on_block_announced_on_source_chain {
                                    callback(block_identifier).await;
                                }

                                purgatory_queue.push(block_identifier.into());
                            },
                            Err(_) => {
                                // TODO: report that for some reason block didn't contain block number
                            }
                        }
                    },
                    None => break Outcome::StreamDropped,
                }
            },
            // every half block time let the purgatory queue check for blocks eligible for expulsion
            // each block will be retained in the purgatory for one source chain block time
            () = sleep(Duration::from_millis(PURGATORY_PERIOD_MILLIS as u64 / 2)) => {
                let num_of_active_block_creation_tasks = ongoing_create_attestation_block_tasks.read().await.keys().len();
                let num_of_blocks_to_expulse = max_num_of_blocks_to_expulse.saturating_sub(num_of_active_block_creation_tasks);

                let expulsed = purgatory_queue.expulse(
                    Some(num_of_blocks_to_expulse)
                );
                // invoke callback if the backpressure was applied
                if expulsed.len() == max_num_of_blocks_to_expulse {
                    if let Some(ref callback) = on_backpressure_applied {
                        callback((num_of_blocks_to_expulse, purgatory_queue.len())).await;
                    }
                }

                let mut outcome = Outcome::None;
                for block in expulsed.into_iter().map(|b| b.block) {
                    // send to block creation task
                    let result = append_to_chain_sender.send(block).map(|()| block);

                    if let Some(ref callback) = on_send_block_to_appending_task_outcome {
                        callback(result).await;
                    }

                    if result.is_err() {
                        outcome = Outcome::SenderError(block); // fatal, exit
                        break;
                    }
                };
                if let Outcome::SenderError(_) = outcome {
                    break outcome;
                }
            },

            _ = cancellation_token.cancelled() => {
                break Outcome::Cancelled;
            },
        }
    };

    if let Some(callback) = on_leaving_block_listener_event_loop {
        callback(outcome.clone()).await;
    }
    // graceful shutdown sequence, wait for ongoing tasks, purge blocks
    if outcome == Outcome::None || outcome == Outcome::Cancelled {
        let expulsed = purgatory_queue.expulse(None);

        for block in expulsed.into_iter().map(|b| b.block) {
            let result = append_to_chain_sender.send(block).map(|()| block);

            if let Some(ref callback) = on_send_block_to_appending_task_outcome {
                callback(result).await;
            }

            if result.is_err() {
                // if receiver was closed on the other side, it's due to forced shutdown
                outcome = Outcome::Forced;
                break;
            }
        }
    }

    if let Some(callback) = on_block_listener_event_loop_left {
        callback(outcome).await;
    }
}
