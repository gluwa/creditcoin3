use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use attestation_blocks_online_builder::AsyncCallbackWithArg;
use attestation_chain::attestation_fragment::AttestationFragment;
use attestation_chain::block::Block;
use attestation_db::json_db::AttestationJsonDB;
use attestation_db::{AttestationDB, AttestationDbError, FullFragment};

#[derive(PartialEq, Clone, Debug)]
pub enum Outcome {
    Cancelled,
    SenderDropped,
    //    StopConditionReached(StopCondition),
}

pub(crate) enum EventLoopKind {
    BlockListenerDbEventLoop,
    HistoricalBlocksCrawlerDbEventLoop,
}

pub struct DbManager {
    cancellation_token: CancellationToken,
    event_loop_join_handle: Option<JoinHandle<()>>,
}

impl DbManager {
    fn new(
        event_loop_kind: EventLoopKind,
        runtime: Arc<Runtime>,
        db: Arc<RwLock<AttestationJsonDB>>,
        block_receiver: UnboundedReceiver<Block>,
        //        stop_condition: StopCondition,
        cancellation_token: CancellationToken,

        block_append_outcome_callback: Option<
            AsyncCallbackWithArg<Result<Block, AttestationDbError>, ()>,
        >,
        full_fragment_append_outcome_callback: Option<
            AsyncCallbackWithArg<Result<Box<AttestationFragment>, AttestationDbError>, ()>,
        >,
        db_manager_task_exitted_callback: Option<AsyncCallbackWithArg<Outcome, ()>>,
    ) -> Self {
        //        let cancellation_token = CancellationToken::new();
        let cancellation_token_cloned = cancellation_token.clone();

        let event_loop_join_handle = Some(runtime.spawn(async move {
            match event_loop_kind {
                EventLoopKind::BlockListenerDbEventLoop => {
                    {
                        block_listener_db_event_loop(
                            block_receiver,
                            db,
                            cancellation_token_cloned,
                            block_append_outcome_callback,
                            full_fragment_append_outcome_callback,
                            db_manager_task_exitted_callback,
                        )
                    }
                    .await
                }

                EventLoopKind::HistoricalBlocksCrawlerDbEventLoop => {
                    {
                        historical_blocks_forward_crawler_db_event_loop(
                            block_receiver,
                            db,
                            cancellation_token_cloned,
                            //                        stop_condition,
                            block_append_outcome_callback,
                            full_fragment_append_outcome_callback,
                            db_manager_task_exitted_callback,
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

    #[allow(dead_code)]
    pub fn shutdown(mut self) -> JoinHandle<()> {
        self.cancellation_token.cancel();
        self.event_loop_join_handle
            .take()
            .expect("not supposed to run twice")
    }
    pub fn wait_for_stop_condition(mut self) -> JoinHandle<()> {
        self.event_loop_join_handle
            .take()
            .expect("not supposed to run twice")
    }
}

#[allow(dead_code)]
async fn historical_blocks_backward_crawler_db_event_loop(
    mut block_receiver: UnboundedReceiver<Block>,
    db: Arc<RwLock<AttestationJsonDB>>,
    cancellation_token: CancellationToken,
    //    stop_condition: StopCondition,
    on_block_append_outcome: Option<AsyncCallbackWithArg<Result<Block, AttestationDbError>, ()>>,
    on_full_fragment_set_outcome: Option<
        AsyncCallbackWithArg<Result<Box<AttestationFragment>, AttestationDbError>, ()>,
    >,
    on_db_manager_task_exitted: Option<AsyncCallbackWithArg<Outcome, ()>>,
) {
    let mut fragment = AttestationFragment::default();

    let outcome = loop {
        tokio::select! {
            res = block_receiver.recv() => match res {
                Some(block) => {
                    let result = fragment.try_append_block(block).map_err(|err| err.into());

                    match result {
                        Ok(head) => {
                            let head = head.clone();
                            match FullFragment::try_from(&fragment) {
                                Ok(full_fragment) => {
                                    let result = db.write().await.set_fragment(full_fragment);

                                    if let Some(ref callback) = on_full_fragment_set_outcome {
                                        callback(result.map(|()| Box::new(fragment.clone()))).await;
                                    }

                                    fragment = AttestationFragment::default();
                                },
                                Err(_) => {
                                    if let Some(ref callback) = on_block_append_outcome {
                                        callback(Ok(head)).await;
                                    }
                                }
                            }
                        },
                        Err(err) => {
                            if let Some(ref callback) = on_block_append_outcome {
                                callback(Err(err)).await;
                            }
                        },
                    }
                },

                None => break Outcome::SenderDropped,
            },
            _ = cancellation_token.cancelled() => break Outcome::Cancelled,
        }
    };

    if let Some(callback) = on_db_manager_task_exitted {
        callback(outcome).await;
    }
}

async fn historical_blocks_forward_crawler_db_event_loop(
    mut block_receiver: UnboundedReceiver<Block>,
    db: Arc<RwLock<AttestationJsonDB>>,
    cancellation_token: CancellationToken,
    //    stop_condition: StopCondition,
    on_block_append_outcome: Option<AsyncCallbackWithArg<Result<Block, AttestationDbError>, ()>>,
    on_full_fragment_set_outcome: Option<
        AsyncCallbackWithArg<Result<Box<AttestationFragment>, AttestationDbError>, ()>,
    >,
    on_db_manager_task_exitted: Option<AsyncCallbackWithArg<Outcome, ()>>,
) {
    let outcome = loop {
        tokio::select! {
            res = block_receiver.recv() => match res {
                Some(block) => {
                    let result = db.write().await.try_append_block(block);
                    match result {
                        Ok(Some(fragment_boxed)) => {
                            if let Some(ref callback) = on_full_fragment_set_outcome {
                                callback(Ok(fragment_boxed.clone())).await;
                            }
                        },
                        Err(AttestationDbError::FragmentAlreadySet(fragment_interval)) => {
//                            Err(AttestationDbError::FragmentAlreadySet(fragment_boxed)) => {
//                            let fragment_boxed_cloned = fragment_boxed.clone();
                            if let Some(ref callback) = on_full_fragment_set_outcome {
                                callback(Err(AttestationDbError::FragmentAlreadySet(fragment_interval))).await;
//                                callback(Err(AttestationDbError::FragmentAlreadySet(fragment_boxed_cloned))).await;
                            }
                        },
                        Ok(None) => {
                            if let Some(ref callback) = on_block_append_outcome {
                                let head = db.read().await.recent_fragment().head().expect("fragment has head").clone();
                                callback(Ok(head)).await;
                            }
                        },

                        Err(err) => {
                            if let Some(ref callback) = on_block_append_outcome {
                                callback(Err(err)).await;
                            }
                        },
                    }
                },

                None => break Outcome::SenderDropped,
            },
            _ = cancellation_token.cancelled() => break Outcome::Cancelled,
        }
    };

    if let Some(callback) = on_db_manager_task_exitted {
        callback(outcome).await;
    }
}

async fn block_listener_db_event_loop(
    mut block_receiver: UnboundedReceiver<Block>,
    db: Arc<RwLock<AttestationJsonDB>>,
    cancellation_token: CancellationToken,

    on_block_append_outcome: Option<AsyncCallbackWithArg<Result<Block, AttestationDbError>, ()>>,
    on_full_fragment_set_outcome: Option<
        AsyncCallbackWithArg<Result<Box<AttestationFragment>, AttestationDbError>, ()>,
    >,
    on_db_manager_task_exitted: Option<AsyncCallbackWithArg<Outcome, ()>>,
) {
    let outcome = loop {
        tokio::select! {
            res = block_receiver.recv() => {
                match res {
                    Some(block) => {
                        let result = db.write().await.try_append_block(block);
                        match result {
                            Ok(Some(fragment_boxed)) => {
                                if let Some(ref callback) = on_full_fragment_set_outcome {
                                    callback(Ok(fragment_boxed)).await;
                                }
                            },
                            Ok(None) => {
                                if let Some(ref callback) = on_block_append_outcome {
                                    let head = db.read().await.recent_fragment().head().expect("fragment has head").clone();
                                    callback(Ok(head)).await;
                                }
                            },
                            Err(err) => {
                                if let Some(ref callback) = on_block_append_outcome {
                                    callback(Err(err)).await;
                                }
                            },
                        }
                    },

                    None => break Outcome::SenderDropped,
                }
            },
            _ = cancellation_token.cancelled() => break Outcome::Cancelled,
        }
    };

    if let Some(callback) = on_db_manager_task_exitted {
        callback(outcome).await;
    }
}

pub struct DbManagerBuilder {
    //    pub struct DbManagerBuilder<P: OnBlockStopConditionPredicate> {
    runtime: Arc<Runtime>,
    db: Arc<RwLock<AttestationJsonDB>>,
    block_receiver: Option<UnboundedReceiver<Block>>,
    //    stop_condition: StopCondition<P>,
    cancellation_token: Option<CancellationToken>,

    block_append_outcome_callback:
        Option<AsyncCallbackWithArg<Result<Block, AttestationDbError>, ()>>,
    //    full_fragment_appended_callback: Option<AsyncCallbackWithArg<u64, ()>>,
    full_fragment_append_outcome_callback:
        Option<AsyncCallbackWithArg<Result<Box<AttestationFragment>, AttestationDbError>, ()>>,
    db_manager_task_exitted_callback: Option<AsyncCallbackWithArg<Outcome, ()>>,
}

impl DbManagerBuilder {
    //    impl<P: OnBlockStopConditionPredicate> DbManagerBuilder<P> {
    pub fn new(
        runtime: Arc<Runtime>,
        db: Arc<RwLock<AttestationJsonDB>>,
        block_receiver: UnboundedReceiver<Block>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            runtime,
            db,
            block_receiver: Some(block_receiver),
            //            stop_condition: Default::default(),
            cancellation_token: Some(cancellation_token),
            //            cancellation_token: Some(CancellationToken::new()),
            block_append_outcome_callback: None,
            full_fragment_append_outcome_callback: None,
            db_manager_task_exitted_callback: None,
        }
    }
    // pub fn with_stop_condition(mut self, stop_condition: StopCondition<P>,) -> Self {
    //     self.stop_condition = stop_condition;
    //     self
    // }

    pub fn build_block_listener_db_manager(&mut self) -> DbManager {
        DbManager::new(
            EventLoopKind::BlockListenerDbEventLoop,
            Arc::clone(&self.runtime),
            Arc::clone(&self.db),
            self.block_receiver.take().expect("called for second time?"),
            //            self.stop_condition,
            self.cancellation_token
                .take()
                .expect("called for second time?"),
            self.block_append_outcome_callback.take(),
            self.full_fragment_append_outcome_callback.take(),
            self.db_manager_task_exitted_callback.take(),
        )
    }

    pub fn build_historical_blocks_crawler_db_manager(&mut self) -> DbManager {
        DbManager::new(
            EventLoopKind::HistoricalBlocksCrawlerDbEventLoop,
            Arc::clone(&self.runtime),
            Arc::clone(&self.db),
            self.block_receiver.take().expect("called for second time?"),
            //            self.stop_condition,
            self.cancellation_token
                .take()
                .expect("called for second time?"),
            self.block_append_outcome_callback.take(),
            self.full_fragment_append_outcome_callback.take(),
            self.db_manager_task_exitted_callback.take(),
        )
    }

    pub fn on_block_append_outcome<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Result<Block, AttestationDbError>, ()>,
        F: Fn(Result<Block, AttestationDbError>) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.block_append_outcome_callback = Some(Arc::new(f));
        self
    }

    pub fn on_full_fragment_set_outcome<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Result<Box<AttestationFragment>, AttestationDbError>, ()>,
        F: Fn(Result<Box<AttestationFragment>, AttestationDbError>) -> BoxFuture<'static, ()>
            + Send
            + Sync
            + 'static,
    {
        self.full_fragment_append_outcome_callback = Some(Arc::new(f));
        self
    }

    pub fn on_db_manager_task_exitted<F>(&mut self, f: F) -> &mut Self
    where
        //        F: AsyncCallbackWithArgTrait<Outcome, ()>,
        F: Fn(Outcome) -> BoxFuture<'static, ()> + Send + Sync + 'static,
    {
        self.db_manager_task_exitted_callback = Some(Arc::new(f));
        self
    }
}
