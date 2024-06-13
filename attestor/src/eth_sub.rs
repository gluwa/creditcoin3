use anyhow::Result;
use eth::{self, subscription::SubscriptionConfig};
use kameo::actor::ActorRef;
use sp_core::H256;
use thiserror::Error;
use tracing::error;

use crate::{
    attestation::{self, Attestor, NewBlock},
    cc3::{self, AttestationSubmit, Client},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Failed to fetch block {0}")]
    FailedToFetchBlock(String),
    #[error("Actor send error {0}")]
    AttestationError(#[from] kameo::error::SendError<NewBlock, attestation::Error>),
    #[error("Attestation submit error {0}")]
    AttestationSubmitError(#[from] kameo::error::SendError<AttestationSubmit<H256>, cc3::Error>),
    #[error("Eth client error {0}")]
    EthClientError(#[from] eth::Error),
}

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn subscribe_to_new_heads(
    eth_client: eth::Client,
    attestor: ActorRef<Attestor>,
    cc3_client: ActorRef<Client>,
    eth_start_block: Option<u64>,
    attestation_interval: u64,
) -> Result<(), Error> {
    // Only create a subscription config if we have a start block
    let config = eth_start_block.map(|eth_start_block| SubscriptionConfig {
        start_block: eth_start_block,
        interval: attestation_interval,
    });

    let mut subscription = eth_client.open_subscription(config).map_err(|e| {
        Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}"))
    })?;

    // Continuously await new blocks and notify the attestor
    loop {
        if let Some(block) = subscription.next().await {
            // TODO: find a way to query receipts on a hardhat node (or some sidecar) https://github.com/NomicFoundation/hardhat/issues/4761
            let receipts = eth_client
                .get_receipts(block.header.number.unwrap_or_default())
                .await?;

            let transactions = eth_client
                .get_transactions(block.header.number.unwrap_or_default())
                .await?;

            let chain_id = eth_client.get_chain_id().await?;

            // let last_digest = cc3_client.send(GetLastDigest { chain_id }).await?;

            // Notify the attestor with a new block
            let attestation = attestor
                .send(NewBlock {
                    chain_id,
                    header_number: block.header.number.unwrap(),
                    header_hash: sp_core::H256(block.header.hash.unwrap().0),
                    transactions,
                    receipts,
                })
                .await?;

            cc3_client.send(AttestationSubmit { attestation }).await?;
        } else {
            error!("Subscription stream ended unexpectedly");
            subscription = eth_client.open_subscription(None).map_err(|e| {
                Error::FailedToSubscribe(format!("Failed to subscribe to new heads on chain: {e}",))
            })?;
        }
    }
}
