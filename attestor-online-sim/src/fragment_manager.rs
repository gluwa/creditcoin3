use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use attestation_blocks_online_builder::AsyncCallbackWithArg;
use attestation_chain::attestation_checkpoints::AttestationCheckpoint;
use attestation_chain::attestation_fragment::{AttestationFragment, AttestationFragmentError};
use attestation_chain::block::Block;

#[derive(PartialEq, Clone, Debug)]
pub enum Outcome {
    Cancelled,
    SenderDropped,
}

#[allow(dead_code)]
pub(crate) enum EventLoopKind {
    BlockListenerEventLoop,
    HistoricalBlocksCrawlerEventLoop,
}

pub struct FragmentManager {
    cancellation_token: CancellationToken,
    event_loop_join_handle: Option<JoinHandle<()>>,
}

impl FragmentManager {
    pub(crate) fn new(
        event_loop_kind: EventLoopKind,
        runtime: Arc<Runtime>,
        block_receiver: UnboundedReceiver<Block>,

        block_append_outcome_callback: Option<
            AsyncCallbackWithArg<Result<Block, AttestationFragmentError>, ()>,
        >,
        on_checkpoint_ready: Option<AsyncCallbackWithArg<AttestationCheckpoint, ()>>,
        task_exitted_callback: Option<AsyncCallbackWithArg<Outcome, ()>>,
    ) -> Self {
        let cancellation_token = CancellationToken::new();
        let cancellation_token_cloned = cancellation_token.clone();

        let event_loop_join_handle = Some(runtime.spawn(async move {
            match event_loop_kind {
                EventLoopKind::BlockListenerEventLoop => {
                    {
                        block_listener_event_loop(
                            block_receiver,
                            cancellation_token_cloned,
                            block_append_outcome_callback,
                            on_checkpoint_ready,
                            task_exitted_callback,
                        )
                    }
                    .await
                }

                EventLoopKind::HistoricalBlocksCrawlerEventLoop => {
                    {
                        historical_blocks_crawler_event_loop(
                            block_receiver,
                            cancellation_token_cloned,
                            block_append_outcome_callback,
                            on_checkpoint_ready,
                            task_exitted_callback,
                        )
                    }
                    .await
                }
            }
        }));

        Self {
            cancellation_token,
            event_loop_join_handle,
        }
    }

    pub fn shutdown(mut self) -> JoinHandle<()> {
        self.cancellation_token.cancel();
        self.event_loop_join_handle
            .take()
            .expect("not supposed to run twice")
    }
}

async fn historical_blocks_crawler_event_loop(
    mut block_receiver: UnboundedReceiver<Block>,
    cancellation_token: CancellationToken,

    on_block_append_outcome: Option<
        AsyncCallbackWithArg<Result<Block, AttestationFragmentError>, ()>,
    >,
    on_checkpoint_ready: Option<AsyncCallbackWithArg<AttestationCheckpoint, ()>>,
    on_exitted: Option<AsyncCallbackWithArg<Outcome, ()>>,
) {
    let mut fragment = AttestationFragment::default();

    let outcome = loop {
        tokio::select! {
            res = block_receiver.recv() => match res {
                Some(block) => {
                    let result = fragment.try_append_block(block);

                    match result {
                        Ok(head)  => {
                            let head = head.clone();
                            if let Some(checkpoint) = fragment.checkpoint() {
                                if let Some(ref callback) = on_checkpoint_ready {
                                    callback(checkpoint).await;
                                }
                                fragment = AttestationFragment::default();
                            } else if let Some(ref callback) = on_block_append_outcome {
                                    //let head = fragment.head().expect("fragment has head").clone();
                                    callback(Ok(head)).await;
                            }
                        },
                        Err(err) => {
                            if let Some(ref callback) = on_block_append_outcome {
                                //let head = fragment.head().expect("fragment has head").clone();
                                //callback(result.map(|()| head)).await;
                                callback(Err(err)).await;
                            }
                        }
                    }
                },

                None => break Outcome::SenderDropped,
            },
            _ = cancellation_token.cancelled() => break Outcome::Cancelled,
        }
    };

    if let Some(callback) = on_exitted {
        callback(outcome).await;
    }
}

async fn block_listener_event_loop(
    mut block_receiver: UnboundedReceiver<Block>,
    cancellation_token: CancellationToken,

    on_block_append_outcome: Option<
        AsyncCallbackWithArg<Result<Block, AttestationFragmentError>, ()>,
    >,
    on_checkpoint_ready: Option<AsyncCallbackWithArg<AttestationCheckpoint, ()>>,
    on_exitted: Option<AsyncCallbackWithArg<Outcome, ()>>,
) {
    let mut fragment = AttestationFragment::default();

    let outcome = loop {
        tokio::select! {
            res = block_receiver.recv() => {
                match res {
                    Some(block) => match fragment.try_append_block(block) {
                        Ok(head) => {
                            let head = head.clone();
                            if let Some(checkpoint) = fragment.checkpoint() {
                                fragment = fragment.next().expect("full fragment can chain next fragment");

                                if let Some(ref callback) = on_checkpoint_ready {
                                    callback(checkpoint).await;
                                }
                            } else if let Some(ref callback) = on_block_append_outcome {
                                    callback(Ok(head)).await;
                            }
                        },
                        Err(err) => {
                            if let Some(ref callback) = on_block_append_outcome {
                                callback(Err(err)).await;
                            }
                        }
                    },

                    None => break Outcome::SenderDropped,
                }
            },
            _ = cancellation_token.cancelled() => break Outcome::Cancelled,
        }
    };

    if let Some(callback) = on_exitted {
        callback(outcome).await;
    }
}

pub struct FragmentManagerBuilder {
    runtime: Arc<Runtime>,
    block_receiver: Option<UnboundedReceiver<Block>>,

    block_append_outcome_callback:
        Option<AsyncCallbackWithArg<Result<Block, AttestationFragmentError>, ()>>,
    checkpoint_ready_callback: Option<AsyncCallbackWithArg<AttestationCheckpoint, ()>>,
    task_exitted_callback: Option<AsyncCallbackWithArg<Outcome, ()>>,
}

impl FragmentManagerBuilder {
    pub fn new(runtime: Arc<Runtime>, block_receiver: UnboundedReceiver<Block>) -> Self {
        Self {
            runtime,
            block_receiver: Some(block_receiver),

            block_append_outcome_callback: None,
            checkpoint_ready_callback: None,
            task_exitted_callback: None,
        }
    }

    pub fn build_block_listener_manager(&mut self) -> FragmentManager {
        FragmentManager::new(
            EventLoopKind::BlockListenerEventLoop,
            Arc::clone(&self.runtime),
            self.block_receiver.take().expect("called for second time?"),
            self.block_append_outcome_callback.take(),
            self.checkpoint_ready_callback.take(),
            self.task_exitted_callback.take(),
        )
    }

    #[allow(dead_code)]
    pub fn build_historical_blocks_crawler_fragment_manager(&mut self) -> FragmentManager {
        FragmentManager::new(
            EventLoopKind::HistoricalBlocksCrawlerEventLoop,
            Arc::clone(&self.runtime),
            self.block_receiver.take().expect("called for second time?"),
            self.block_append_outcome_callback.take(),
            self.checkpoint_ready_callback.take(),
            self.task_exitted_callback.take(),
        )
    }

    pub fn on_block_append_outcome<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Result<Block, AttestationFragmentError>, ()>
        F: Fn(Result<Block, AttestationFragmentError>) -> BoxFuture<'static, ()>
            + Send
            + Sync
            + 'static,
    {
        self.block_append_outcome_callback = Some(Arc::new(f));
        self
    }

    pub fn on_checkpoint_ready<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<AttestationCheckpoint, ()>
        F: Fn(AttestationCheckpoint) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.checkpoint_ready_callback = Some(Arc::new(f));
        self
    }

    pub fn on_task_exitted<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Outcome, ()>
        F: Fn(Outcome) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.task_exitted_callback = Some(Arc::new(f));
        self
    }
}
