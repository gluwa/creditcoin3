use anyhow::Result;
use attestor_primitives::{Attestation, ChainKey};
use eth::{self, subscription::SubscriptionConfig};
use kameo::actor::ActorRef;
use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
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
}

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn subscribe_to_new_heads(
    eth_client: eth::Client,
    attestor: ActorRef<Attestor>,
    sender: Sender<Option<Attestation<H256>>>,
    eth_start_block: u64,
    attestation_interval: u64,
    chain_key: ChainKey,
) -> Result<(), Error> {
    let config = SubscriptionConfig {
        start_block: eth_start_block,
        interval: attestation_interval,
    };

    let mut subscription = eth_client.open_subscription(Some(config)).map_err(|e| {
        Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}"))
    })?;

    loop {
        if let Some(block) = subscription.next().await {
            // Continuously await new blocks and notify the attestor
            let attestation = attestor.send((chain_key, block)).await?;

            // Send an attestation back on the channel
            sender
                .send(attestation)
                .await
                .map_err(|e| Error::FailedToSubscribe(e.to_string()))?;
        } else {
            warn!("Subscription stream ended unexpectedly");
            info!("Subscribing to latest blockheads now");
            subscription = eth_client.open_subscription(None).map_err(|e| {
                Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}",))
            })?;
        }
    }
}
