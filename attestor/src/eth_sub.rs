use anyhow::Result;
use eth::{self, subscription::SubscriptionConfig};
use kameo::actor::ActorRef;
use sp_core::H256;
use thiserror::Error;
use tracing::error;

use crate::{
    attestation::{self, Attestor},
    cc3::{AttestationSubmit, Client},
};
use eth::OrderedBlock;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Failed to fetch block {0}")]
    FailedToFetchBlock(String),
    #[error("Actor send error {0}")]
    AttestationError(#[from] kameo::error::SendError<OrderedBlock, attestation::Error>),
    #[error("Attestation submit error {0}")]
    AttestationSubmitError(
        #[from] kameo::error::SendError<AttestationSubmit<H256>, cc_client::Error>,
    ),
    #[error("Eth client error {0}")]
    EthClientError(#[from] eth::Error),
}

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn subscribe_to_new_heads(
    eth_client: eth::Client,
    attestor: ActorRef<Attestor>,
    cc3_client: ActorRef<Client>,
    eth_start_block: u64,
    attestation_interval: u64,
) -> Result<(), Error> {
    let config = SubscriptionConfig {
        start_block: eth_start_block,
        interval: attestation_interval,
    };

    let mut subscription = eth_client.open_subscription(Some(config)).map_err(|e| {
        Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}"))
    })?;

    // Continuously await new blocks and notify the attestor
    loop {
        if let Some(block) = subscription.next().await {
            let attestation = attestor.send(block).await?;

            match cc3_client.send(AttestationSubmit { attestation }).await {
                Ok(()) => {}
                Err(e) => {
                    error!("Failed to submit attestation: {:?}", e);
                    return subscription.cancel().map_err(|e| {
                        Error::FailedToSubscribe(format!(
                            "Failed to cancel subscription to new heads on chain: {e}"
                        ))
                    });
                }
            }
        } else {
            error!("Subscription stream ended unexpectedly");
            subscription = eth_client.open_subscription(None).map_err(|e| {
                Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}",))
            })?;
        }
    }
}
