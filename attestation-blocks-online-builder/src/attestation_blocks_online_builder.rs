use futures::future::BoxFuture;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::mpsc::{error::SendError, unbounded_channel};
use tokio::sync::RwLock;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::build_attestation_chain_task::{
    build_attestation_chain_task, Outcome as ChainBuildTaskOutcome,
};
use crate::check_connectivity::check_connectivity_task;
use crate::create_attestation_block_task::CreateAttestationBlockError;
use crate::ethereum_block_listener::Outcome as BlockListenerOutcome;
use crate::network_failures_resilience::{resiliency_queue_event_loop, ContinuityHandle};
use attestation_chain::block::Block;
//use crate::{SourceChainBlockIdentifier, AsyncCallbackTrait, AsyncCallback, AsyncCallbackWithArgTrait, AsyncCallbackWithArg};
use crate::source_blocks_provider::SourceBlocksProvider;
use crate::{AsyncCallback, AsyncCallbackWithArg, SourceChainBlockIdentifier};
//type BlockListenerOutcome = crate::ethereum_block_listener::Outcome;

#[derive(Default)]
pub struct ShutdownErrors {
    pub block_listener: Option<JoinError>,
    pub resiliency_queue: Option<JoinError>,
    pub check_connectivity: Option<JoinError>,
    pub build_chain: Option<JoinError>,
}

impl ShutdownErrors {
    pub fn into_option(self) -> Option<Self> {
        if self.block_listener.is_some()
            || self.resiliency_queue.is_some()
            || self.check_connectivity.is_some()
            || self.build_chain.is_some()
        {
            Some(self)
        } else {
            None
        }
    }
}

pub struct InstanceBuilder {
    source_chain_api_server_url: Arc<str>,
    cache_dir: Option<Arc<str>>,
    runtime: Arc<tokio::runtime::Runtime>,
    block_stream_provider: Option<SourceBlocksProvider>,
    max_num_of_blocks_to_retrieve: usize,
    cancellation_token: Option<CancellationToken>,
    //    reset_resiliency_queue_receiver: Option<UnboundedReceiver<()>>,
    create_attestation_block_outcome_callback:
        Option<AsyncCallbackWithArg<Result<u64, CreateAttestationBlockError>, ()>>,
    block_ready_callback: Option<AsyncCallbackWithArg<Block, ()>>,
    late_block_dropped_callback: Option<AsyncCallbackWithArg<u64, ()>>,
    block_announced_on_source_chain_callback:
        Option<AsyncCallbackWithArg<SourceChainBlockIdentifier, ()>>,
    send_block_to_appending_task_outcome_callback: Option<
        AsyncCallbackWithArg<
            Result<SourceChainBlockIdentifier, SendError<SourceChainBlockIdentifier>>,
            (),
        >,
    >,
    leaving_block_listener_event_loop_callback:
        Option<AsyncCallbackWithArg<BlockListenerOutcome, ()>>,
    block_listener_event_loop_left_callback: Option<AsyncCallbackWithArg<BlockListenerOutcome, ()>>,
    backpressure_applied_callback: Option<AsyncCallbackWithArg<(usize, usize), ()>>,
    announced_block_is_being_processed_callback: Option<AsyncCallbackWithArg<u64, ()>>,
    waiting_to_finish_creating_block_task_callback: Option<AsyncCallbackWithArg<u64, ()>>,
    attestation_chain_build_task_exitted_callback:
        Option<AsyncCallbackWithArg<ChainBuildTaskOutcome, ()>>,
    retry_retrieve_block_callback: Option<AsyncCallbackWithArg<(u64, String, u64), ()>>,
    toggle_connection_mode_callback: Option<AsyncCallbackWithArg<bool, ()>>,
    checking_connectivity_callback: Option<AsyncCallback<()>>,
}

impl InstanceBuilder {
    pub fn new(
        source_chain_api_server_url: &str,
        cache_dir: Option<&str>,
        runtime: Arc<tokio::runtime::Runtime>,
        block_stream_provider: SourceBlocksProvider,
        max_num_of_blocks_to_retrieve: usize,
        cancellation_token: CancellationToken,
        //        reset_resiliency_queue_receiver: Option<UnboundedReceiver<()>>,
    ) -> Self {
        Self {
            source_chain_api_server_url: Arc::from(source_chain_api_server_url),
            cache_dir: cache_dir.map(Arc::from),
            runtime,
            block_stream_provider: Some(block_stream_provider),
            max_num_of_blocks_to_retrieve,
            cancellation_token: Some(cancellation_token),
            // reset_resiliency_queue_receiver: reset_resiliency_queue_receiver
            //                                     .or_else(|| {
            //                                         let (dummy_tx, dummy_rx) = unbounded_channel::<()>();
            //                                         Some(dummy_rx)
            //                                     }),
            create_attestation_block_outcome_callback: None,
            block_ready_callback: None,
            late_block_dropped_callback: None,
            block_announced_on_source_chain_callback: None,
            send_block_to_appending_task_outcome_callback: None,
            leaving_block_listener_event_loop_callback: None,
            block_listener_event_loop_left_callback: None,
            backpressure_applied_callback: None,
            announced_block_is_being_processed_callback: None,
            waiting_to_finish_creating_block_task_callback: None,
            attestation_chain_build_task_exitted_callback: None,
            retry_retrieve_block_callback: None,
            toggle_connection_mode_callback: None,
            checking_connectivity_callback: None,
        }
    }

    pub fn build<const SOURCE_BLOCK_TIME_MILLIS: u128>(
        &mut self,
    ) -> AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS> {
        AttestationChainOnlineBuilder::<SOURCE_BLOCK_TIME_MILLIS>::new(
            Arc::clone(&self.source_chain_api_server_url),
            self.cache_dir.clone(),
            Arc::clone(&self.runtime),
            self.block_stream_provider.take().expect("called twice?"),
            self.max_num_of_blocks_to_retrieve,
            self.cancellation_token.take().expect("called twice?"),
            //            self.reset_resiliency_queue_receiver.take().expect("called twice?"),
            self.create_attestation_block_outcome_callback.take(),
            self.block_ready_callback.take(),
            self.late_block_dropped_callback.take(),
            self.block_announced_on_source_chain_callback.take(),
            self.send_block_to_appending_task_outcome_callback.take(),
            self.leaving_block_listener_event_loop_callback.take(),
            self.block_listener_event_loop_left_callback.take(),
            self.backpressure_applied_callback.take(),
            self.announced_block_is_being_processed_callback.take(),
            self.waiting_to_finish_creating_block_task_callback.take(),
            self.attestation_chain_build_task_exitted_callback.take(),
            self.retry_retrieve_block_callback.take(),
            self.toggle_connection_mode_callback.take(),
            self.checking_connectivity_callback.take(),
        )
    }

    pub fn on_create_attestation_block_outcome<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Result<u64, CreateAttestationBlockError>, ()>
        F: Fn(Result<u64, CreateAttestationBlockError>) -> BoxFuture<'static, ()>
            + Send
            + Sync
            + 'static,
    {
        self.create_attestation_block_outcome_callback = Some(Arc::new(f));
        self
    }

    pub fn on_block_ready<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Block, ()>
        F: Fn(Block) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.block_ready_callback = Some(Arc::new(f));
        self
    }
    pub fn on_late_block_dropped<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(u64) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.late_block_dropped_callback = Some(Arc::new(f));
        self
    }

    pub fn on_block_announced_on_source_chain<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<SourceChainBlockIdentifier, ()>
        F: Fn(SourceChainBlockIdentifier) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.block_announced_on_source_chain_callback = Some(Arc::new(f));
        self
    }

    pub fn on_send_block_to_appending_task_outcome<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Result<SourceChainBlockIdentifier, SendError<SourceChainBlockIdentifier>>, ()>
        F: Fn(
                Result<SourceChainBlockIdentifier, SendError<SourceChainBlockIdentifier>>,
            ) -> BoxFuture<'static, ()>
            + Send
            + Sync
            + 'static,
    {
        self.send_block_to_appending_task_outcome_callback = Some(Arc::new(f));
        self
    }

    pub fn on_leaving_block_listener_event_loop<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<BlockListenerOutcome, ()>
        F: Fn(BlockListenerOutcome) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.leaving_block_listener_event_loop_callback = Some(Arc::new(f));
        self
    }

    pub fn on_block_listener_event_loop_left<F>(&mut self, f: F) -> &mut Self
    where
        F: Fn(BlockListenerOutcome) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.block_listener_event_loop_left_callback = Some(Arc::new(f));
        self
    }

    pub fn on_backpressure_applied<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<(usize, usize), ()>
        F: Fn((usize, usize)) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.backpressure_applied_callback = Some(Arc::new(f));
        self
    }

    pub fn on_attestation_chain_build_task_exitted<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<ChainBuildTaskOutcome, ()>
        F: Fn(ChainBuildTaskOutcome) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.attestation_chain_build_task_exitted_callback = Some(Arc::new(f));
        self
    }

    pub fn on_retry_retrieve_block<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<(u64, String, u64), ()>
        F: Fn((u64, String, u64)) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.retry_retrieve_block_callback = Some(Arc::new(f));
        self
    }

    pub fn on_toggle_connection_mode<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<bool, ()>
        F: Fn(bool) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.toggle_connection_mode_callback = Some(Arc::new(f));
        self
    }

    pub fn on_checking_connectivity<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackTrait<()>
        F: Fn() -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.checking_connectivity_callback = Some(Arc::new(f));
        self
    }

    pub fn on_announced_block_is_being_processed<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<u64, ()>
        F: Fn(u64) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.announced_block_is_being_processed_callback = Some(Arc::new(f));
        self
    }

    pub fn on_waiting_to_finish_creating_block_task<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<u64, ()>
        F: Fn(u64) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.waiting_to_finish_creating_block_task_callback = Some(Arc::new(f));
        self
    }
}

pub struct AttestationChainOnlineBuilder<const SOURCE_BLOCK_TIME_MILLIS: u128> {
    runtime: Arc<tokio::runtime::Runtime>,
    cancellation_token: CancellationToken,
    force_shutdown_token: CancellationToken,
    block_listener_join_handle: Option<JoinHandle<()>>,
    resiliency_queue_event_loop_join_handle: Option<JoinHandle<()>>,
    check_connectivity_join_handle: Option<JoinHandle<()>>,
    build_chain_join_handle: Option<JoinHandle<()>>,
    //    purgatory_queue: Arc<RwLock<BlockPurgatoryQueue<SOURCE_BLOCK_TIME_MILLIS>>>,
    pub shutdown_errors: ShutdownErrors,
}

impl<const SOURCE_BLOCK_TIME_MILLIS: u128> AttestationChainOnlineBuilder<SOURCE_BLOCK_TIME_MILLIS> {
    fn new(
        source_chain_api_server_url: Arc<str>,
        cache_dir: Option<Arc<str>>,
        runtime: Arc<tokio::runtime::Runtime>,
        mut block_stream_provider: SourceBlocksProvider,
        max_num_of_blocks_to_retrieve: usize,
        cancellation_token: CancellationToken,
        //        reset_resiliency_queue_receiver: UnboundedReceiver<()>,
        create_attestation_block_outcome_callback: Option<
            AsyncCallbackWithArg<Result<u64, CreateAttestationBlockError>, ()>,
        >,
        block_ready_callback: Option<AsyncCallbackWithArg<Block, ()>>,
        late_block_dropped_callback: Option<AsyncCallbackWithArg<u64, ()>>,
        block_announced_on_source_chain_callback: Option<
            AsyncCallbackWithArg<SourceChainBlockIdentifier, ()>,
        >,
        send_block_to_appending_task_outcome_callback: Option<
            AsyncCallbackWithArg<
                Result<SourceChainBlockIdentifier, SendError<SourceChainBlockIdentifier>>,
                (),
            >,
        >,
        leaving_block_listener_event_loop_callback: Option<
            AsyncCallbackWithArg<BlockListenerOutcome, ()>,
        >,
        block_listener_event_loop_left_callback: Option<
            AsyncCallbackWithArg<BlockListenerOutcome, ()>,
        >,
        backpressure_applied_callback: Option<AsyncCallbackWithArg<(usize, usize), ()>>,
        announced_block_is_being_processed_callback: Option<AsyncCallbackWithArg<u64, ()>>,
        waiting_to_finish_creating_block_task_callback: Option<AsyncCallbackWithArg<u64, ()>>,
        attestation_chain_build_task_exitted_callback: Option<
            AsyncCallbackWithArg<ChainBuildTaskOutcome, ()>,
        >,
        retry_retrieve_block_callback: Option<AsyncCallbackWithArg<(u64, String, u64), ()>>,
        toggle_connection_mode_callback: Option<AsyncCallbackWithArg<bool, ()>>,
        checking_connectivity_callback: Option<AsyncCallback<()>>,
    ) -> Self {
        let (append_to_chain_sender, append_to_chain_receiver) =
            unbounded_channel::<SourceChainBlockIdentifier>();
        let (resiliency_sender, resiliency_receiver) = unbounded_channel::<ContinuityHandle>();

        //        let cancellation_token = CancellationToken::new();
        let cancellation_token_cloned = cancellation_token.clone();
        let check_connectivity_cancellation_token = cancellation_token.clone();

        let force_shutdown_token = CancellationToken::new();
        let force_shutdown_token_cloned = force_shutdown_token.clone();

        let ongoing_create_attestation_block_tasks = Arc::new(RwLock::new(HashMap::new()));
        let ongoing_create_attestation_block_tasks_cloned =
            Arc::clone(&ongoing_create_attestation_block_tasks);

        let runtime_cloned = Arc::clone(&runtime);

        let disconnected = Arc::new(AtomicBool::new(false));
        let disconnected_cloned = Arc::clone(&disconnected);

        let toggle_connection_mode_callback_cloned = toggle_connection_mode_callback.clone();

        //        let purgatory_queue = Arc::new(RwLock::new(BlockPurgatoryQueue::<12000u128>::new()));
        //        let purgatory_queue = Arc::new(RwLock::new(BlockPurgatoryQueue::<SOURCE_BLOCK_TIME_MILLIS>::new()));
        //        let purgatory_queue_cloned = Arc::clone(&purgatory_queue);

        let connectivity_urls: [&str; 2] =
            ["clients3.google.com:80", "detectportal.firefox.com:80"];

        let check_connectivity_join_handle = Some(runtime.spawn(async move {
            check_connectivity_task(
                disconnected,
                connectivity_urls[0],
                500,
                500,
                check_connectivity_cancellation_token,
                toggle_connection_mode_callback_cloned,
                checking_connectivity_callback,
            )
            .await
        }));

        let build_chain_join_handle = Some(runtime.spawn(async move {
            build_attestation_chain_task(
                source_chain_api_server_url,
                cache_dir,
                runtime_cloned,
                append_to_chain_receiver, // receiver of the expulsed blocks from the listener
                resiliency_sender, // sender of the attestation blocks to the resiliency queue
                disconnected_cloned,
                force_shutdown_token_cloned,
                ongoing_create_attestation_block_tasks_cloned,
                // hook callbacks
                announced_block_is_being_processed_callback,
                create_attestation_block_outcome_callback,
                waiting_to_finish_creating_block_task_callback,
                attestation_chain_build_task_exitted_callback,
                retry_retrieve_block_callback,
                toggle_connection_mode_callback,
            )
            .await
        }));

        //        let reset_resiliency_queue_receiver = block_stream_provider.reset_resiliency_queue_receiver().unwrap();
        let reset_resiliency_queue_receiver = block_stream_provider
            .reset_resiliency_queue_receiver()
            .unwrap_or_else(|| {
                let (_, dummy_rx) = unbounded_channel::<()>();
                dummy_rx
            });

        let resiliency_queue_event_loop_join_handle = Some(runtime.spawn(async move {
            resiliency_queue_event_loop(
                resiliency_receiver,
                reset_resiliency_queue_receiver,
                // hook callbacks
                block_ready_callback,
                late_block_dropped_callback,
            )
            .await
        }));

        let block_listener_join_handle = Some(runtime.spawn(async move {
            // let (tx, rx) = unbounded_channel::<EthersBlock>();
            block_stream_provider
                .run_listener(
                    append_to_chain_sender, // sender to build_attestation_chain_task
                    ongoing_create_attestation_block_tasks, // how many block creation tasks are currently active
                    cancellation_token_cloned,
                    max_num_of_blocks_to_retrieve, // backpressure parameter
                    //                purgatory_queue,
                    // hook callbacks
                    block_announced_on_source_chain_callback,
                    send_block_to_appending_task_outcome_callback,
                    leaving_block_listener_event_loop_callback,
                    block_listener_event_loop_left_callback,
                    backpressure_applied_callback,
                )
                .await;
        }));

        Self {
            runtime,
            cancellation_token,
            force_shutdown_token,
            block_listener_join_handle,
            resiliency_queue_event_loop_join_handle,
            check_connectivity_join_handle,
            build_chain_join_handle,
            //            purgatory_queue,
            shutdown_errors: Default::default(),
        }
    }

    pub fn gracefully_shutdown(mut self) -> JoinHandle<Self> {
        self.runtime.clone().spawn(async move {
            self.cancellation_token.cancel();

            self.shutdown_errors.block_listener = self
                .block_listener_join_handle
                .take()
                .expect("called for the second time?")
                .await
                .err();
            self.shutdown_errors.resiliency_queue = self
                .resiliency_queue_event_loop_join_handle
                .take()
                .expect("called for the second time?")
                .await
                .err();
            self.shutdown_errors.check_connectivity = self
                .check_connectivity_join_handle
                .take()
                .expect("called for the second time?")
                .await
                .err();
            self.shutdown_errors.build_chain = self
                .build_chain_join_handle
                .take()
                .expect("called for the second time?")
                .await
                .err();

            self
        })
    }

    pub fn partially_clone_for_forced_shutdown(&self) -> Self {
        Self {
            runtime: self.runtime.clone(),
            cancellation_token: self.cancellation_token.clone(),
            force_shutdown_token: self.force_shutdown_token.clone(),
            //            purgatory_queue: Arc::clone(&self.purgatory_queue),
            block_listener_join_handle: None,
            resiliency_queue_event_loop_join_handle: None,
            check_connectivity_join_handle: None,
            build_chain_join_handle: None,
            shutdown_errors: Default::default(),
        }
    }

    pub fn force_shutdown(self) -> Result<(), JoinError> {
        if self.block_listener_join_handle.is_some() {
            self.runtime
                .clone()
                .block_on(self.gracefully_shutdown())
                .map(|this| this.force_shutdown_token.cancel())
        } else {
            self.force_shutdown_token.cancel();
            Ok(())
        }
    }

    // pub fn purgatory_queue(&self) -> Arc<RwLock<BlockPurgatoryQueue<SOURCE_BLOCK_TIME_MILLIS, SourceChainBlockIdentifier>>> {
    //     Arc::clone(&self.purgatory_queue)
    // }
}
