use crate::{Client, OrderedBlock};
use alloy::providers::Provider;
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::Error;

#[async_trait]
pub trait BlockSubscription: Send + Sync {
    fn cancel(&self);
    async fn next(&mut self) -> Result<Option<OrderedBlock>, Error>;
}

const BUFFER_SIZE: usize = 100;

/// `NewBlockSubscription` is a struct that references to a receiving end of a channel where blocks are pushed upon
/// It subscribes to the head of the chain and pushes new blocks to the channel
#[derive(Debug)]
struct NewBlockSubscription {
    receiver: mpsc::Receiver<OrderedBlock>,
    handle: JoinHandle<Result<(), Error>>,
}

impl Drop for NewBlockSubscription {
    fn drop(&mut self) {
        self.cancel()
    }
}

#[async_trait]
impl BlockSubscription for NewBlockSubscription {
    /// Cancel the subscription
    fn cancel(&self) {
        // Cancel the subscription task
        debug!("Canceling subscription");
        self.handle.abort();
    }

    /// Get the next block from the channel
    async fn next(&mut self) -> Result<Option<OrderedBlock>, Error> {
        match self.receiver.recv().await {
            Some(block) => Ok(Some(block)),
            None => {
                warn!("Channel closed; no more blocks will be received");
                Ok(None)
            }
        }
    }
}

/// Subscribe to the latest heads of the chain
/// This function returns a `BlockSubscription` trait object
fn subscribe_latest_heads(
    client: Client,
    interval: u64,
) -> Result<Box<dyn BlockSubscription>, Error> {
    let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

    let client = client.clone();
    let handle = tokio::spawn(async move {
        let provider = client.get_ws().await?;
        let subscription = provider.subscribe_blocks().await?;
        // Open stream
        let mut stream = subscription.into_stream();

        loop {
            if let Some(header) = stream.next().await {
                let block_number = header.number;

                debug!("Received block: {}", block_number);
                // Skip blocks that are not at the interval
                if block_number % interval != 0 {
                    debug!("Skipping block: {}", block_number);
                    continue;
                }

                let block = client.get_block(block_number).await?;

                debug!("Sending block({}) to channel", block_number);
                sender.send(block).await?;
            } else {
                info!("Subscription stream ended");
                return Err(Error::EndOfSubscription);
            }
        }
    });

    Ok(Box::new(NewBlockSubscription { receiver, handle }))
}

/// `BlockFetcher` is a struct that fetches blocks from a given height with a given interval
struct BlockFetcher {
    pub client: Client,
    pub config: SubscriptionConfig,
    pub interval: u64,
}

impl BlockFetcher {
    pub fn new(client: Client, config: SubscriptionConfig, interval: u64) -> Self {
        Self {
            client,
            config,
            interval,
        }
    }
}

#[async_trait]
impl BlockSubscription for BlockFetcher {
    fn cancel(&self) {}

    async fn next(&mut self) -> Result<Option<OrderedBlock>, Error> {
        // If we reached the end block, return EndOfSubscription error
        if self.config.start_block >= self.config.end_block {
            return Err(Error::EndOfSubscription);
        }

        info!(
            "Blockfetcher: Fetching block at height: {}",
            self.config.start_block
        );
        // Get the block at the current height
        let block = self.client.get_block(self.config.start_block).await?;

        // Increment the height
        self.config.start_block += self.interval;

        Ok(Some(block))
    }
}

///   Subscription configuration
///   - `start_block`: The block number to start the subscription from
///   - `end_block`: The block number to end the subscription
///     This is only relevant when fetching blocks from a specific height
///     An interval is provided to speed up the fetching process
pub struct SubscriptionConfig {
    pub start_block: u64,
    pub end_block: u64,
}

impl Client {
    // Open a subscription to the chain
    // This function returns a `BlockSubscription` trait object
    // - `config`: Subscription configuration
    // - `interval`: The interval to fetch blocks
    // If no configuration is provided, it will subscribe to the latest heads
    // If a configuration is provided, it will fetch blocks from a specific height with a given interval & switch to latest heads if it's all caught up
    pub fn open_subscription(
        &self,
        config: Option<SubscriptionConfig>,
        interval: u64,
    ) -> Result<Box<dyn BlockSubscription>, Error> {
        let client = self.clone();
        if let Some(config) = config {
            Ok(Box::new(BlockFetcher::new(client, config, interval)))
        } else {
            Ok(subscribe_latest_heads(client, interval)?)
        }
    }
}
