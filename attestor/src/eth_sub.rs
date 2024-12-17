use anyhow::Result;
use attestor_primitives::{Attestation, ChainKey};
use eth::{self, subscription::SubscriptionConfig};
use kameo::actor::ActorRef;
use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc::{error::SendError, Sender};
use tracing::{error, info, warn};

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

    info!(
        "Starting block fetcher for chain: {} from block {} to block {}",
        chain_key, config.start_block, config.end_block
    );
    let last_block_height = eth_client.get_last_block().await?;
    info!("Last block height: {}", last_block_height);

    if config.end_block > last_block_height {
        warn!("End block is greater than current block, setting end block to current block");
        config.end_block = last_block_height;
    }

    // Open a block fetcher subscription
    // Providing the config will fetch historical blocks
    let mut subscription = eth_client.open_subscription(Some(config), attestation_interval)?;

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
                    info!("Nore more blocks to fetch, stopping blockfetcher");
                    subscription = eth_client.open_subscription(None, attestation_interval)?;
                } else {
                    error!("Error fetching block: {:?}", e);
                    return Err(Error::FailedToSubscribe(e.to_string()));
                }
            }
        }
    }
}
