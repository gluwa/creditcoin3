use alloy::{
    providers::{Provider, ProviderBuilder, RootProvider},
    pubsub::PubSubFrontend,
    rpc::{
        client::WsConnect,
        types::eth::{Block, BlockId, BlockNumberOrTag, BlockTransactions},
    },
};
use anyhow::Result;
use thiserror::Error;
use tracing::{error, info};

pub use alloy::core::primitives::Address;

pub mod subscription;
pub mod transaction;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get block {0}")]
    FailedToGetBlock(u64),
    #[error("Failed to get receipts")]
    FailedToGetReceipts,
    #[error("Failed to get chain id")]
    FailedToGetChainId,
}

#[derive(Debug, Clone)]
pub struct Client {
    provider: RootProvider<PubSubFrontend>,
}

impl Client {
    pub async fn new(url: impl Into<String>) -> Result<Self> {
        // Create a provider.
        let ws = WsConnect::new(url);
        let provider = ProviderBuilder::new().on_ws(ws).await?;

        Ok(Self { provider })
    }

    pub async fn get_block(&self, number: u64) -> Result<Block, Error> {
        let block = self
            .provider
            .get_block(BlockId::Number(BlockNumberOrTag::Number(number)), true)
            .await
            .map_err(|e| {
                error!("Failed to get block: {:?}", e);
                Error::FailedToGetBlock(number)
            })?;

        if let Some(block) = block {
            Ok(block)
        } else {
            Err(Error::FailedToGetBlock(number))
        }
    }

    pub async fn get_receipts(&self, number: u64) -> Result<Vec<transaction::Receipt>, Error> {
        let receipts = self
            .provider
            .get_block_receipts(alloy::rpc::types::eth::BlockNumberOrTag::Number(number))
            .await
            .map_err(|e| {
                error!("Failed to get receipts: {:?}", e);
                Error::FailedToGetReceipts
            })?;

        let receipts = receipts
            .into_iter()
            .flatten()
            .map(transaction::Receipt)
            .collect();

        Ok(receipts)
    }

    pub async fn get_transactions(
        &self,
        number: u64,
    ) -> Result<Vec<transaction::Transaction>, Error> {
        let block = self.get_block(number).await?;

        let transactions = if let BlockTransactions::Full(tx) = block.transactions {
            tx.into_iter().map(transaction::Transaction).collect()
        } else {
            info!("No full tx");
            vec![]
        };

        Ok(transactions)
    }

    pub async fn get_chain_id(&self) -> Result<u64, Error> {
        self.provider.get_chain_id().await.map_err(|e| {
            error!("Failed to get chain id: {:?}", e);
            Error::FailedToGetChainId
        })
    }
}
