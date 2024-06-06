use crate::Client;
use alloy::{providers::Provider, rpc::types::eth::Block, transports::TransportErrorKind};
use anyhow::Result;
use futures_util::StreamExt;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

const BUFFER_SIZE: usize = 100;

/// `NewBlockSubscription` is a struct that references to a receiving end of a channel where blocks are pushed upon
#[derive(Debug)]
pub struct NewBlockSubscription {
    receiver: mpsc::Receiver<Block>,
    handle: JoinHandle<Result<(), Error>>,
}

impl NewBlockSubscription {
    /// Cancel the subscription
    pub fn cancel(&self) -> Result<()> {
        // Cancel the subscription task
        debug!("Canceling subscription");
        self.handle.abort();
        Ok(())
    }

    /// Get the next proof
    pub async fn next(&mut self) -> Option<Block> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Failed to fetch block {0}")]
    FailedToFetchBlock(String),
    #[error("Ethereum RPC error {0}")]
    EthError(#[from] alloy::transports::RpcError<TransportErrorKind>),
    #[error("client error {0}")]
    ClientError(#[from] anyhow::Error),
}

impl Client {
    pub fn subscribe_latest_heads(&self) -> Result<NewBlockSubscription, Error> {
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);
        let provider = self.provider.clone();

        let handle = tokio::spawn(async move {
            let subscription = provider.subscribe_blocks().await?;
            let mut stream = subscription.into_stream();

            loop {
                if let Some(block) = stream.next().await {
                    let hash = block.header.hash.ok_or(Error::FailedToFetchBlock(
                        block.header.hash.unwrap_or_default().to_string(),
                    ))?;
                    info!("New block header: {:?}", hash);

                    let block = provider
                        .get_block_by_hash(hash, true)
                        .await?
                        .ok_or(Error::FailedToFetchBlock(hash.to_string()))?;

                    sender.send(block).await.ok();
                }
            }
        });

        Ok(NewBlockSubscription { receiver, handle })
    }
}
