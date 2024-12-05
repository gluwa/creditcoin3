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
pub async fn attest_to_heads(
    eth_client: eth::Client,
    attestor: ActorRef<Attestor>,
    sender: Sender<Option<Attestation<H256>>>,
    eth_start_block: u64,
    eth_target_block: u64,
    chain_key: ChainKey,
    attestation_interval: u64,
) -> Result<(), Error> {
    let config = SubscriptionConfig {
        start_block: eth_start_block,
        end_block: eth_target_block,
    };

    let mut subscription = eth_client
        .open_subscription(Some(config), attestation_interval)
        .map_err(|e| {
            Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}"))
        })?;

    loop {
        match subscription.next().await {
            Ok(next) => {
                if let Some(block) = next {
                    // Continuously await new blocks and notify the attestor
                    let attestation = attestor.send((chain_key, block)).await?;

                    // Send an attestation back on the channel
                    sender
                        .send(attestation)
                        .await
                        .map_err(|e| Error::FailedToSubscribe(e.to_string()))?;
                } else {
                    warn!("We shouldn't get here");
                }
            }
            Err(e) => {
                if matches!(e, eth::Error::EndOfSubscription) {
                    return Ok(());
                } else if matches!(e, eth::Error::FailedToGetBlock(_)) {
                    info!("Block doesn't exist yet, opening a sub to new heads");
                    subscription = eth_client
                        .open_subscription(None, attestation_interval)
                        .map_err(|e| {
                            Error::FailedToSubscribe(format!(
                                "Failed to subscribe to new heads on chain: {e}"
                            ))
                        })?;
                } else {
                    return Err(Error::EthClientError(e));
                }
            }
        }
    }
}
