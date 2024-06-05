use alloy::{
    providers::{Provider, ProviderBuilder},
    rpc::client::WsConnect,
    rpc::types::eth::BlockTransactions,
    transports::TransportErrorKind,
};
use anyhow::Result;
use eth;
use futures_util::StreamExt;
use kameo::actor::ActorRef;
use sp_core::H256;
use thiserror::Error;
use tracing::{debug, error, info};

use crate::{
    attestation::{self, Attestor, NewBlock},
    cc3::{self, AttestationSubmit, Client, GetLastDigest},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Failed to fetch block {0}")]
    FailedToFetchBlock(String),
    #[error("Ethereum RPC error {0}")]
    EthError(#[from] alloy::transports::RpcError<TransportErrorKind>),
    #[error("Actor send error {0}")]
    AttestationError(#[from] kameo::error::SendError<NewBlock, attestation::Error>),
    #[error("Attestation submit error {0}")]
    AttestationSubmitError(#[from] kameo::error::SendError<AttestationSubmit<H256>, cc3::Error>),
    #[error("Get last digest error {0}")]
    FetchDigestError(#[from] kameo::error::SendError<GetLastDigest, cc3::Error>),
}

/// Subscribes to new heads on a chain configured by the url, it also takes an attestor which is an Actor
/// where we can send the new block to in order to start the attestation cycle
pub async fn subscribe_to_new_heads(
    url: &str,
    attestor: ActorRef<Attestor>,
    cc3_client: ActorRef<Client>,
) -> Result<(), Error> {
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

            // TODO: find a way to query receipts on a hardhat node (or some sidecar) https://github.com/NomicFoundation/hardhat/issues/4761
            let receipts = provider
                .get_block_receipts(alloy::rpc::types::eth::BlockNumberOrTag::Number(
                    block.header.number.unwrap(),
                ))
                .await?;

            let receipts = receipts.into_iter().flatten().map(eth::Receipt).collect();

            let transactions = if let BlockTransactions::Full(tx) = block.transactions {
                tx.into_iter().map(eth::Transaction).collect()
            } else {
                info!("No full tx");
                vec![]
            };

            let chain_id = provider.get_chain_id().await?;

            let last_digest = cc3_client.send(GetLastDigest { chain_id }).await?;

            // Notify the attestor with a new block
            let attestation = attestor
                .send(NewBlock {
                    chain_id,
                    header_number: block.header.number.unwrap(),
                    header_hash: sp_core::H256(block.header.hash.unwrap().0),
                    last_digest,
                    transactions,
                    receipts,
                })
                .await?;

            cc3_client.send(AttestationSubmit { attestation }).await?;
        } else {
            panic!("no block");
        }
    }
}
