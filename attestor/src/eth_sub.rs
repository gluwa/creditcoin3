use anyhow::Result;
use attestor_primitives::{Attestation, ChainKey};
use eth::{self, subscription::SubscriptionConfig};
use sp_core::H256;
use std::time::Duration;
use tokio::{sync::mpsc::Sender, time::sleep};
use tracing::{debug, error, info};

use crate::error::Error;

/// Attest to heads of the source chain
/// This function will open a subscription to the source chain and continuously await new blocks.
pub async fn attest_to_heads(
    eth_client: eth::Client,
    sender: Sender<Attestation<H256>>,
    eth_start_block: u64,
    eth_target_block: u64,
    chain_key: ChainKey,
    attestation_interval: u64,
) -> Result<(), Error> {
    let mut config = SubscriptionConfig {
        start_block: eth_start_block,
        end_block: eth_target_block,
    };

    let last_block_height = eth_client.get_last_block().await?;
    debug!("Last block height: {}", last_block_height);

    // If the start block is greater than the last block height minus the interval, open a new subscription to latest heads
    // It means we can just follow the source chain and we don't need to fetch historical blocks
    let mut subscription = if config.start_block
        >= last_block_height.saturating_sub(attestation_interval)
    {
        info!("Opening subscription to new heads");
        eth_client.open_subscription(None, attestation_interval)?
    } else {
        // If the end block of the configuration is larger than the actual block height we set it to the last block height
        // This causes the historical block fetcher to stop at the last block instead of continuing to fetch blocks which don't exist
        if config.end_block > last_block_height {
            debug!("End block is greater than current block, setting end block to current block");
            config.end_block = last_block_height;
        }

        // If the last block height is greater than the end block, adjust it
        if last_block_height > config.end_block {
            debug!("Last block height is greater than end block, setting end block to last block height");
            config.end_block = last_block_height;
        }

        info!(
            "🔎 Crawling historical blocks, from: {} to: {}",
            config.start_block, config.end_block
        );
        // Providing the config will fetch historical blocks
        eth_client.open_subscription(Some(config), attestation_interval)?
    };

    loop {
        match subscription.next().await {
            Ok(next) => {
                if let Some(block) = next {
                    // Continuously await new blocks and notify the attestor
                    let attestation = crate::attestation::create(chain_key, &block);

                    debug!("Sending attestation: {:?}", attestation.round());
                    // Send an attestation back on the channel
                    sender.send(attestation).await?;

                    // Sleep for a bit to avoid spamming the chain
                    sleep(Duration::from_millis(100)).await;
                } else {
                    return Err(Error::FailedToSubscribe("No block received".to_string()));
                }
            }
            Err(e) => {
                if matches!(e, eth::Error::EndOfSubscription) {
                    debug!("Done crawling historical blocks, switching to new heads subscription");
                    subscription = eth_client.open_subscription(None, attestation_interval)?;
                } else {
                    error!("Error fetching block: {:?}", e);
                    return Err(Error::FailedToSubscribe(e.to_string()));
                }
            }
        }
    }
}
