#![allow(clippy::type_complexity)]
#![allow(clippy::result_unit_err)]

use crate::create_attestation_block_task::CreateAttestationBlockError;
use crate::ethereum_block_listener::{ethereum_block_listener, Outcome};
use crate::{
    AsyncCallbackWithArg, HistoricalBlocksProvider, NextHistoricalBlockInjector,
    SourceChainBlockIdentifier,
};
use ethers::providers::{Middleware, Provider, Ws};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

pub(crate) trait SourceChainBlockStream:
    StreamExt<Item = Self::SourceBlock> + Unpin + Send + Sync
{
    type SourceBlock: TryInto<SourceChainBlockIdentifier> + Send + Sync;
}

pub enum SourceBlocksProvider {
    NewBlocksWsSubscription(Provider<Ws>),
    HistoricalBlocksCrawlerProvider(HistoricalBlocksProvider),
}

impl SourceBlocksProvider {
    pub fn new_blocks_subscription_provider(provider: Provider<Ws>) -> Self {
        Self::NewBlocksWsSubscription(provider)
    }
    pub fn historical_blocks_provider() -> Self {
        Self::HistoricalBlocksCrawlerProvider(HistoricalBlocksProvider::new())
    }

    pub fn start(
        &mut self,
        start_block: SourceChainBlockIdentifier,
    ) -> Result<SourceChainBlockIdentifier, ()> {
        match self {
            Self::NewBlocksWsSubscription(_) => Ok(start_block),
            Self::HistoricalBlocksCrawlerProvider(provider) => {
                provider.start(start_block).map_err(|_| ())
            }
        }
    }
    pub fn reset_resiliency_queue_receiver(&mut self) -> Option<UnboundedReceiver<()>> {
        match self {
            Self::NewBlocksWsSubscription(_) => None,
            Self::HistoricalBlocksCrawlerProvider(_provider) => None,
            //            Self::HistoricalBlocksCrawlerProvider(provider) => provider.reset_resiliency_queue_receiver(),
        }
    }
    pub fn block_injector(&mut self) -> Option<NextHistoricalBlockInjector> {
        match self {
            Self::NewBlocksWsSubscription(_) => None,
            Self::HistoricalBlocksCrawlerProvider(provider) => provider.block_injector(),
        }
    }

    pub async fn run_listener(
        self,
        append_to_chain_sender: UnboundedSender<SourceChainBlockIdentifier>,
        ongoing_create_attestation_block_tasks: Arc<
            RwLock<HashMap<u64, JoinHandle<Result<u64, CreateAttestationBlockError>>>>,
        >,
        cancellation_token: CancellationToken,
        max_num_of_blocks_to_expulse: usize, // backpressure-related parameter
        //        purgatory_queue: Arc<RwLock<BlockPurgatoryQueue<12000u128>>>,
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
        match self {
            Self::NewBlocksWsSubscription(ref provider) => {
                ethereum_block_listener::<{ crate::SOURCE_BLOCK_TIME_MILLIS }, _>(
                    Box::new(provider.subscribe_blocks().await.unwrap()),
                    append_to_chain_sender, // sender to build_attestation_chain_task
                    ongoing_create_attestation_block_tasks, // how many block creation tasks are currently active
                    cancellation_token,
                    max_num_of_blocks_to_expulse, // backpressure parameter
                    //                    purgatory_queue,
                    // hook callbacks
                    on_block_announced_on_source_chain,
                    on_send_block_to_appending_task_outcome,
                    on_leaving_block_listener_event_loop,
                    on_block_listener_event_loop_left,
                    on_backpressure_applied,
                )
                .await
            }
            Self::HistoricalBlocksCrawlerProvider(mut provider) => {
                ethereum_block_listener::<0u128, _>(
                    Box::new(provider.subscribe().expect("can subscribe only once")),
                    append_to_chain_sender, // sender to build_attestation_chain_task
                    ongoing_create_attestation_block_tasks, // how many block creation tasks are currently active
                    cancellation_token,
                    max_num_of_blocks_to_expulse, // backpressure parameter
                    //                    purgatory_queue,
                    // hook callbacks
                    on_block_announced_on_source_chain,
                    on_send_block_to_appending_task_outcome,
                    on_leaving_block_listener_event_loop,
                    on_block_listener_event_loop_left,
                    on_backpressure_applied,
                )
                .await
            }
        }
    }
}
