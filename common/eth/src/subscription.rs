use crate::Client;
use alloy::{providers::Provider, rpc::types::eth::Block};
use anyhow::Result;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::Error;

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

impl Client {
    pub fn subscribe_latest_heads(&self) -> Result<NewBlockSubscription, Error> {
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);
        let provider = self.provider.clone();

        let handle = tokio::spawn(async move {
            let subscription = provider.subscribe_blocks().await?;
            let mut stream = subscription.into_stream();

            loop {
                if let Some(block) = stream.next().await {
                    let hash = block.header.hash.ok_or(Error::FailedToGetBlock(
                        block.header.number.unwrap_or_default(),
                    ))?;
                    info!("New block header: {:?}", hash);

                    let block = provider.get_block_by_hash(hash, true).await?.ok_or(
                        Error::FailedToGetBlock(block.header.number.unwrap_or_default()),
                    )?;

                    sender.send(block).await.ok();
                }
            }
        });

        Ok(NewBlockSubscription { receiver, handle })
    }

    pub async fn subscribe_from_head_with_interval(
        &self,
        header_number: u64,
        interval: u64,
    ) -> Result<NewBlockSubscription, Error> {
        tracing::info!(
            "Subscribing from block number: {} with interval {}",
            header_number,
            interval
        );
        let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

        // create a for loop that gets the block by number and sends it to the receiver
        // the loop should start from the header_number
        let client = self.clone();
        let handle = tokio::spawn(async move {
            let mut header_number = header_number;
            info!("Subscribing from block number: {}", header_number);
            loop {
                let block = client.get_block(header_number).await?;

                sender.send(block).await.ok();
                header_number += interval;
            }
        });

        Ok(NewBlockSubscription { receiver, handle })
    }
}
