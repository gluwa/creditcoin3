use crate::print_with_timestamp;
use attestation_blocks_online_builder::SOURCE_BLOCK_TIME_MILLIS;
use attestation_blocks_online_builder::{
    AttestationChainOnlineBuilder, InstanceBuilder, SourceBlocksProvider,
};
use attestation_chain::block::{Block, MaybeCreatedFromEmpty};
use colored::Colorize;
use ethers::providers::{Provider, Ws};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_util::sync::CancellationToken;

pub(crate) fn create_attestation_blocks_builder_instance(
    source_chain_api_server_url: &str,
    wss_url: &str,
    block_cache_dir: Option<&str>,
    runtime: Arc<Runtime>,
    max_num_of_blocks_to_retrieve: usize,
    cancel_on_fatal_failure: CancellationToken,
) -> (
    AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS>,
    UnboundedReceiver<Block>,
) {
    println!("connecting to: {}", wss_url);
    let wss_provider = runtime.block_on(Provider::<Ws>::connect(wss_url)).unwrap();
    let block_stream_provider =
        SourceBlocksProvider::new_blocks_subscription_provider(wss_provider);
    println!("connected");

    let (db_block_sender, db_block_receiver) = unbounded_channel::<Block>();

    let cancel_on_fatal_failure_cloned = cancel_on_fatal_failure.clone();
    let cancel_on_exitted = cancel_on_fatal_failure.clone();

    let attestation_chain_builder = InstanceBuilder::new(
        source_chain_api_server_url,
        block_cache_dir,
        Arc::clone(&runtime),
        block_stream_provider,
        max_num_of_blocks_to_retrieve,
        CancellationToken::new(),
    )
    .on_block_announced_on_source_chain(move |block_number| {
        Box::pin(async move {
            print_with_timestamp(format!("new block {} born on source chain, sending to purgatory", block_number).bold());
        })
    })
    .on_announced_block_is_being_processed(move |block_number| {
        Box::pin(async move {
//            print_with_timestamp(format!("build_chain: recv block: {}", block_number,).bright_cyan().into());
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

        Box::pin(async move {
            match outcome {
                Ok(block_number) => {
                    print_with_timestamp(format!("attestation block created: {} ===>", block_number).on_bright_magenta());
                },
                Err(err) => {
                    println!("{}", format!("failed to create attestation block: {:?}", err).bright_red());
                    println!("{}", "fatal: can't ensure continuity, exitting".to_owned().red());

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

            Box::pin(async move {
                if block.created_from_empty() {
                    println!("{}", 
                        format!("wow, block {} is empty on source chain, this is rare", block.n())
                            .bold().on_bright_yellow()
                    );
                }
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
            println!("{}", format!("attestation chain build task exitted, outcome: {outcome:?}").yellow());
            cancel_on_exitted.cancel();
        })
    })
    .build::<{SOURCE_BLOCK_TIME_MILLIS}>();

    (attestation_chain_builder, db_block_receiver)
}
