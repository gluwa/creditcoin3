use anyhow::Result;
use attestor_primitives::{Attestation, ChainKey};
use eth::{self, subscription::SubscriptionConfig};
use kameo::actor::ActorRef;
use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc::{error::SendError, Sender};
use tracing::{debug, error, info};

use crate::attestation::{self, Attestor};
use eth::OrderedBlock;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Actor send error {0}")]
    AttestationError(#[from] kameo::error::SendError<(ChainKey, OrderedBlock), attestation::Error>),
    #[error("Eth client error {0}")]
    EthClientError(#[from] eth::Error),
    #[error("Send error {0}")]
    SendError(#[from] SendError<Option<Attestation<H256>>>),
}

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn attest_to_heads(
    eth_client: eth::Client,
    attestor: ActorRef<Attestor>,
    sender: Sender<Option<Attestation<H256>>>,
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

    // If the start block is greater than the last block height, open a new subscription to latest heads
    // It means we can just follow the source chain and we don't need to fetch historical blocks
    let mut subscription = if config.start_block >= last_block_height {
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
            "Crawling historical blocks, from: {} to: {}",
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
                    let attestation = attestor.send((chain_key, block)).await?;

                    // Send an attestation back on the channel
                    sender.send(attestation).await?;
                } else {
                    return Err(Error::FailedToSubscribe("No block received".to_string()));
                }
            }
            Err(e) => {
                if matches!(e, eth::Error::EndOfSubscription) {
                    info!("Done crawling historical blocks, switching to new heads subscription");
                    subscription = eth_client.open_subscription(None, attestation_interval)?;
                } else {
                    error!("Error fetching block: {:?}", e);
                    return Err(Error::FailedToSubscribe(e.to_string()));
                }
            }
        }
    }
}
