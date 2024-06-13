use crate::Client;
use alloy::{providers::Provider, rpc::types::eth::Block};
use anyhow::Result;
use async_trait::async_trait;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::Error;

#[async_trait]
pub trait BlockSubscription: Send + Sync {
    fn cancel(&self) -> Result<()>;
    async fn next(&mut self) -> Option<Block>;
}

const BUFFER_SIZE: usize = 100;

/// `NewBlockSubscription` is a struct that references to a receiving end of a channel where blocks are pushed upon
/// It subscribes to the head of the chain and pushes new blocks to the channel
#[derive(Debug)]
struct NewBlockSubscription {
    receiver: mpsc::Receiver<Block>,
    handle: JoinHandle<Result<(), Error>>,
}

#[async_trait]
impl BlockSubscription for NewBlockSubscription {
    /// Cancel the subscription
    fn cancel(&self) -> Result<()> {
        // Cancel the subscription task
        debug!("Canceling subscription");
        self.handle.abort();
        Ok(())
    }

    /// Get the next block from the channel
    async fn next(&mut self) -> Option<Block> {
        // Receive the next proof from the channel
        self.receiver.recv().await
    }
}

/// Subscribe to the latest heads of the chain
/// This function returns a `BlockSubscription` trait object
fn subscribe_latest_heads(client: Client) -> Result<Box<dyn BlockSubscription>, Error> {
    let (sender, receiver) = mpsc::channel(BUFFER_SIZE);

    let client = client.clone();
    let handle = tokio::spawn(async move {
        let subscription = client.provider.subscribe_blocks().await?;
        let mut stream = subscription.into_stream();

        loop {
            if let Some(block) = stream.next().await {
                let block_number = block.header.number.unwrap_or_default();

                info!("Received block number: {}", block_number);

                let block = client.get_block(block_number).await?;

                sender.send(block).await.ok();
            }
        }
    });

    Ok(Box::new(NewBlockSubscription { receiver, handle }))
}

/// `BlockFetcher` is a struct that fetches blocks from a given height with a given interval
struct BlockFetcher {
    pub client: Client,
    pub from_height: u64,
    pub interval: u64,
}

impl BlockFetcher {
    pub fn new(client: Client, from_height: u64, interval: u64) -> Self {
        Self {
            client,
            from_height,
            interval,
        }
    }
}

#[async_trait]
impl BlockSubscription for BlockFetcher {
    fn cancel(&self) -> Result<()> {
        debug!("Canceling subscription");
        Ok(())
    }

    async fn next(&mut self) -> Option<Block> {
        let block = self.client.get_block(self.from_height).await.ok();
        self.from_height += self.interval;

        block
    }
}

/// Subscription configuration
/// - `start_block`: The block number to start the subscription from
/// - `interval`: The interval to fetch blocks
/// This is only relevant when fetching blocks from a specific height
/// An interval is provided to speed up the fetching process
pub struct SubscriptionConfig {
    pub start_block: u64,
    pub interval: u64,
}

impl Client {
    /// Open a subscription to the chain
    /// This function returns a `BlockSubscription` trait object
    /// - `config`: Subscription configuration
    /// If no configuration is provided, it will subscribe to the latest heads
    /// If a configuration is provided, it will fetch blocks from a specific height with a given interval & switch to latest heads if it's all caught up
    pub fn open_subscription(
        &self,
        config: Option<SubscriptionConfig>,
    ) -> Result<Box<dyn BlockSubscription>> {
        let client = self.clone();
        if let Some(config) = config {
            Ok(Box::new(BlockFetcher::new(
                client,
                config.start_block,
                config.interval,
            )))
        } else {
            Ok(subscribe_latest_heads(client)?)
        }
    }
}
