use alloy::{
    providers::{Provider, ProviderBuilder},
    rpc::client::WsConnect,
    transports::TransportErrorKind,
};
use anyhow::Result;
use futures_util::StreamExt;
use kameo::ActorRef;
use thiserror::Error;
use tracing::{debug, error, info};

use crate::attestation::{Attestor, NewBlock};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Failed to fetch block {0}")]
    FailedToFetchBlock(String),
    #[error("Ethereum RPC error {0}")]
    EthError(#[from] alloy::transports::RpcError<TransportErrorKind>),
    #[error("Actor send error {0}")]
    SendError(#[from] kameo::SendError<NewBlock, anyhow::Error>),
}

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn subscribe_to_new_heads(url: &str, attestor: ActorRef<Attestor>) -> Result<(), Error> {
    // Create a provider.
    let ws = WsConnect::new(url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;

    // Subscribe to blocks.
    let subscription = provider.subscribe_blocks().await?;

    debug!("subscription for new chain heads started...");

    let mut stream = subscription.into_stream();
    // Continuously await new blocks and notify the attestor
    loop {
        if let Some(block) = stream.next().await {
            let hash = block.header.hash.ok_or(Error::FailedToFetchBlock(
                block.header.hash.unwrap_or_default().to_string(),
            ))?;
            info!("New block header: {:?}", hash);

            let block = provider
                .get_block_by_hash(hash, true)
                .await?
                .ok_or(Error::FailedToFetchBlock(hash.to_string()))?;

            // Notify the attestor with a new block
            let _ = attestor.send(NewBlock { block }).await?;
        } else {
            panic!("no block");
        }
    }
}
