use crate::print_with_timestamp;
use crate::StopCondition;
use attestation_blocks_online_builder::SOURCE_BLOCK_TIME_MILLIS;
use attestation_blocks_online_builder::{
    AttestationChainOnlineBuilder, InstanceBuilder, SourceBlocksProvider,
};
use attestation_chain::block::{Block, MaybeCreatedFromEmpty};
use colored::Colorize;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::signal;
use tokio::sync::mpsc::{unbounded_channel, Receiver, UnboundedReceiver};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub(crate) enum InstanceCreationError {
    Cancelled,
    DidntReceiveKickoffFragment,
    Other(String),
}

pub(crate) async fn create_historical_blocks_crawler_instance(
    source_chain_api_server_url: &str,
    block_cache_dir: Option<&str>,
    runtime: Arc<Runtime>,
    max_num_of_blocks_to_retrieve: usize,
    cancel_on_fatal_failure: CancellationToken,
    mut crawler_kickoff_fragment_ready_rx: Receiver<u64>,
    stop_condition: StopCondition,
) -> Result<
    (
        AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
        UnboundedReceiver<Block>,
    ),
    InstanceCreationError,
> {
    println!(
        "historical block crawler will use {}",
        source_chain_api_server_url
    );

    let cancellation_token = CancellationToken::new();
    let cancellation_token_cloned = cancellation_token.clone();
    let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();
    let cancel_on_exitted = cancel_on_fatal_failure.clone();

    //    runtime.block_on(async {
    tokio::select! {
            _ = signal::ctrl_c() => Err(InstanceCreationError::Cancelled),

            start_block = crawler_kickoff_fragment_ready_rx.recv() => {
                let start_block = start_block.ok_or(InstanceCreationError::DidntReceiveKickoffFragment)?;
    //                start_interval = crawler_kickoff_fragment_ready_rx.recv() => {
                // let start_block = start_interval
                //                     .ok_or(InstanceCreationError::DidntReceiveKickoffFragment)?
                //                     .tail();
                println!("{}", format!("historical block crawler starting from block {}", start_block).bright_green().bold());

                let mut block_stream_provider = SourceBlocksProvider::historical_blocks_provider();
                block_stream_provider
                    .start(Into::<u64>::into(start_block).into())
                    .map_err(|_| InstanceCreationError::Other("expected to be able to inject a start block number to historical blocks stream".to_owned()))?;

                let (db_block_sender, db_block_receiver) = unbounded_channel::<Block>();

                let block_injector = block_stream_provider
                                        .block_injector()
                                        .ok_or(InstanceCreationError::Other("historical block provider expected to provide block injector".to_owned()))?;
                let block_injector = Arc::new(RwLock::new(block_injector));

                let attestation_chain_builder = InstanceBuilder::new(
                    source_chain_api_server_url,
                    block_cache_dir,
                    Arc::clone(&runtime),
                    block_stream_provider,
                    max_num_of_blocks_to_retrieve,
                    cancellation_token,
                )
                .on_block_announced_on_source_chain(move |block_number| {
                    Box::pin(async move {
                        print_with_timestamp(format!("historical block {} to be retrieved from source chain", block_number).bold());
                    })
                })
                .on_announced_block_is_being_processed(move |block_number| {
                    Box::pin(async move {
    //                        print_with_timestamp(format!("build_chain: recv block: {}", block_number,).bright_cyan());
                        print_with_timestamp(format!("retrieving block {block_number}, building merkle trees...").into());
                    })
                })
                .on_retry_retrieve_block(move |(block_number, error_msg, retrial_period)| {
                    Box::pin(async move {
                        println!("{}", format!("block {block_number} retrieval failure: {error_msg:?}").bright_red());
                        println!("{}", format!("will retry after {} ms", retrial_period).yellow());
                    })
                })
                .on_checking_connectivity(move || {
                    Box::pin(async move {
                        println!("{}", "checking connectivity...".yellow());
                    })
                })
                .on_toggle_connection_mode(move |connected| {
                    Box::pin(async move {
                        println!("{}", format!("toggled {} mode", if connected {"connected"} else {"disconnected"}).on_bright_yellow());
                    })
                })
                .on_create_attestation_block_outcome(move |outcome| {
                    let cancel_on_fatal_failure = cancel_on_fatal_failure.clone();
                    let block_injector = Arc::clone(&block_injector);
                    let cancellation_token = cancellation_token_cloned.clone();
                    let stop_condition = stop_condition.clone();

                    Box::pin(async move {
                        match outcome {
                            Ok(block_number) => {
                                print_with_timestamp(format!("<=== attestation block created: {}", block_number).on_bright_magenta());

                                if let StopCondition::OnBlockReached(ref p) = stop_condition {
                                    if p(block_number) {
                                        cancellation_token.cancel();
                                        return;
                                    }
                                }
                                match block_injector.write().await.on_block_appended(block_number).await {
                                    Ok(_) => (),
                                    Err(err) => {
                                        print_with_timestamp(
                                            format!("historical block injector error: {err:?}").red()
                                        );
                                    },
                                }
                            },
                            Err(err) => {
                                println!("{}", format!("failed to create attestation block: {:?}", err).bright_red());
                                println!("{}", "fatal: can't ensure continuity, exitting".red());

                                cancel_on_fatal_failure.cancel();
                            },
                        }
                    })
                })
                .on_waiting_to_finish_creating_block_task(move |block_number| {
                    Box::pin(async move {
                        println!("{}", format!("waiting for finishing block {block_number} creation task...").yellow());
                    })
                })
                .on_send_block_to_appending_task_outcome(move |outcome| {
                    Box::pin(async move {
                        outcome
                            .map(|block_number| {
                                print_with_timestamp(format!("expulsed from purgatory: {}", block_number).into());
                            })
                            .unwrap_or_else(|err| {
                                println!("{}", format!("failed to send block to appending task: {:?}", err).bright_red());
                                println!("{}", "leaving event loop".bright_red());
                            });
                    })
                })
                .on_leaving_block_listener_event_loop(move |outcome| {
                    Box::pin(async move {
                        println!("{}", format!("block listener is leaving event loop, reason: {outcome:?}").yellow());
                        println!("flushing blocks from purgatory queue");
                    })
                })
                .on_block_listener_event_loop_left(move |outcome| {
                    Box::pin(async move {
                        println!("{}", format!("block listener exitted, outcome: {outcome:?}").yellow());
                    })
                })
                .on_backpressure_applied(move |(n_blocks, purgatory_len)| {
                    Box::pin(async move {
                        println!("{}",
                            format!("backpressure applied, {n_blocks} expulsed, there are still {purgatory_len} blocks in purgatory").yellow());
                    })
                })
                .on_block_ready(
                    move |block| {
                        let db_block_sender = db_block_sender.clone();
                        let cancel_on_fatal_failure = cancel_on_fatal_failure_cloned.clone();

                        if block.created_from_empty() {
                            println!("{}",
                                format!("wow, block {} is empty on source chain, this is rare", block.n())
                                    .bold().on_bright_yellow()
                            );
                        }
                        Box::pin(async move {
                            match db_block_sender.send(block) {
                                Ok(_) => (),
                                Err(err) => {
                                    println!("{}", format!("fatal: unable to send block to db manager: {err:?}, exitting").red());
                                    cancel_on_fatal_failure.cancel();
                                },
                            }
                        })
                    }
                )
                .on_late_block_dropped(move |block_number| {
                    Box::pin(async move {
                        println!("{}", format!("late block dropped {}", block_number).bold());
                    })
                })            
                .on_attestation_chain_build_task_exitted(move |outcome| {
                    let cancel_on_exitted = cancel_on_exitted.clone();

                    Box::pin(async move {
                        println!("{}", format!("historical blocks crawler task exitted, outcome: {outcome:?}").yellow());
                        cancel_on_exitted.cancel();
                    })
                })
                .build::<{SOURCE_BLOCK_TIME_MILLIS}>();

                Ok((attestation_chain_builder, db_block_receiver))
            },
        }
    //    })
}
