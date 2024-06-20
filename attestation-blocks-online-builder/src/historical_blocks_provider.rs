use crate::{SourceChainBlockIdentifier, SourceChainBlockStream};
use attestation_chain::attestation_checkpoints::AttestationInterval;
use core::pin::Pin;
use ethereum_types::U256;
use futures::task::{Context, Poll};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub struct HistoricalBlocksProvider {
    stream: Option<HistoricalBlocksStream>,
    block_injector: Option<NextHistoricalBlockInjector>,
}

impl HistoricalBlocksProvider {
    pub fn new() -> Self {
        let (tx, rx) = unbounded_channel::<SourceChainBlockIdentifier>();

        Self {
            stream: Some(HistoricalBlocksStream { rx }),
            block_injector: Some(NextHistoricalBlockInjector {
                watermark: 0.into(),
                tx,
            }),
        }
    }

    pub fn start(
        &self,
        start_block: SourceChainBlockIdentifier,
    ) -> Result<SourceChainBlockIdentifier, HistoricalBlocksProviderError> {
        self.block_injector
            .as_ref()
            .expect("started twice?")
            .inject_block_identifier(start_block)
    }

    pub fn subscribe(&mut self) -> Option<HistoricalBlocksStream> {
        self.stream.take()
    }

    pub fn block_injector(&mut self) -> Option<NextHistoricalBlockInjector> {
        self.block_injector.take()
    }
}
impl Default for HistoricalBlocksProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct NextHistoricalBlockInjector {
    watermark: U256,
    tx: UnboundedSender<SourceChainBlockIdentifier>,
}

impl NextHistoricalBlockInjector {
    pub async fn on_block_appended(
        &mut self,
        block_number: U256,
    ) -> Result<SourceChainBlockIdentifier, HistoricalBlocksProviderError> {
        //        pub async fn on_block_appended(&mut self, block: &Block) -> Result<SourceChainBlockIdentifier, HistoricalBlocksProviderError> {
        //        self.inject_block_identifier((block.n() + 1).into())
        use tokio::time::{sleep, Duration};

        if self.watermark == 0.into() {
            self.watermark = block_number;
            for _ in 0..4 {
                self.watermark += 1.into();
                self.inject_block_identifier(self.watermark.into())?;

                sleep(Duration::from_millis(700)).await;
            }
        }
        self.watermark = core::cmp::max(self.watermark, block_number) + 1;
        self.inject_block_identifier(self.watermark.into())
    }

    pub fn on_fragment_set(
        &self,
        interval: AttestationInterval,
    ) -> Result<SourceChainBlockIdentifier, HistoricalBlocksProviderError> {
        let tail = interval.tail();
        self.inject_block_identifier((tail + 1).into())
    }
}

impl NextHistoricalBlockInjector {
    fn inject_block_identifier(
        &self,
        block_number: SourceChainBlockIdentifier,
    ) -> Result<SourceChainBlockIdentifier, HistoricalBlocksProviderError> {
        self.tx
            .send(block_number)
            .map_err(|_| HistoricalBlocksProviderError::BlockListenerIsDown)
            .map(|_| block_number)
    }
}

#[derive(Debug, Clone)]
pub enum HistoricalBlocksProviderError {
    BlockListenerIsDown,
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
impl SourceChainBlockStream for HistoricalBlocksStream {
    type SourceBlock = SourceChainBlockIdentifier;
}
