use tokio::sync::mpsc::{unbounded_channel, UnboundedSender, UnboundedReceiver};
use futures::task::{Context, Poll};
use core::pin::Pin;
use attestation_chain::{CHECKPOINT_INTERVAL};
use attestation_chain::block::Block;
use attestation_chain::attestation_fragment::AttestationFragment;
use attestation_chain::attestation_checkpoints::AttestationInterval;
use crate::{SourceChainBlockStream, SourceChainBlockIdentifier};

pub struct BackwardHistoricalBlocksProvider {
    stream: Option<HistoricalBlocksStream>,
    block_injector: Option<BackwardNextHistoricalBlockInjector>,
    reset_resiliency_queue_receiver: Option<UnboundedReceiver<()>>,
}

impl BackwardHistoricalBlocksProvider {
    pub fn new() -> Self {
        let (tx, rx) = unbounded_channel::<SourceChainBlockIdentifier>();
        let (reset_resiliency_queue_sender, reset_resiliency_queue_receiver) = unbounded_channel::<()>();

        Self {
            stream: Some(HistoricalBlocksStream {rx}),
            block_injector: Some(BackwardNextHistoricalBlockInjector {
                tx,
                reset_resiliency_queue_sender,
            }),
            reset_resiliency_queue_receiver: Some(reset_resiliency_queue_receiver),
        }
    }

    pub fn start(&self, start_block: SourceChainBlockIdentifier) -> Result<SourceChainBlockIdentifier, BackwardHistoricalBlocksProviderError> {
        self.block_injector
            .as_ref()
            .expect("ran twice?")
            .inject_block_identifier(start_block)
    }

    pub fn subscribe(&mut self) -> Option<HistoricalBlocksStream> {
        self.stream.take()
    }

    pub fn reset_resiliency_queue_receiver(&mut self) -> Option<UnboundedReceiver<()>> {
        self.reset_resiliency_queue_receiver.take()
    }

    pub fn block_injector(&mut self) -> Option<BackwardNextHistoricalBlockInjector> {
        self.block_injector.take()
    }
}

#[derive(Clone)]
pub struct BackwardNextHistoricalBlockInjector {
    tx: UnboundedSender<SourceChainBlockIdentifier>,
    reset_resiliency_queue_sender: UnboundedSender<()>,
}

impl BackwardNextHistoricalBlockInjector {
    pub fn on_block_appended(&self, block: &Block) -> Result<SourceChainBlockIdentifier, BackwardHistoricalBlocksProviderError> {
        self.inject_block_identifier((block.n() + 1).into())
    }

    pub fn on_fragment_set(&self, interval: AttestationInterval) -> Result<SourceChainBlockIdentifier, BackwardHistoricalBlocksProviderError> {
        self.reset_resiliency_queue_sender
            .send(())
            .map_err(|_| BackwardHistoricalBlocksProviderError::ResiliencyQueueIsDown)
            .and_then(|_| {
                Ok(interval.prev().ok_or(BackwardHistoricalBlocksProviderError::GenesisReached)?.tail())
            })
            .and_then(|block_number| self.inject_block_identifier(block_number.into()))
    }
}

impl BackwardNextHistoricalBlockInjector {
    fn inject_block_identifier(&self, block_number: SourceChainBlockIdentifier) -> Result<SourceChainBlockIdentifier, BackwardHistoricalBlocksProviderError> {
        self.tx
            .send(block_number)
            .map_err(|_| BackwardHistoricalBlocksProviderError::BlockListenerIsDown)
            .map(|_| block_number)
    }
}

#[derive(Debug, Clone)]
pub enum BackwardHistoricalBlocksProviderError {
    BlockListenerIsDown,
    ResiliencyQueueIsDown,
    GenesisReached,
}

pub struct HistoricalBlocksStream {
    rx: UnboundedReceiver<SourceChainBlockIdentifier>,
}

impl futures_util::stream::Stream for HistoricalBlocksStream {
    type Item = SourceChainBlockIdentifier;

    fn poll_next(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(ctx)
    }
}
impl SourceChainBlockStream for HistoricalBlocksStream{
    type SourceBlock = SourceChainBlockIdentifier;
}
