use alloy::{
    providers::{Provider, ProviderBuilder, RootProvider},
    pubsub::PubSubFrontend,
    rpc::{
        client::WsConnect,
        types::eth::{Block, BlockId, BlockNumberOrTag, BlockTransactions},
    },
    transports::TransportErrorKind,
};
use anyhow::Result;
use thiserror::Error;
use tracing::{error, info};
use utils::block_item_traits::BlockItemIdentifier;
use crate::transaction::{BlockItem, Transaction, Receipt};

pub use alloy::core::primitives::Address;

pub mod subscription;
pub mod transaction;

pub type AlloyTransaction = alloy::rpc::types::eth::Transaction;
pub type AlloyTransactionReceipt = alloy::rpc::types::eth::TransactionReceipt;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get block {0}")]
    FailedToGetBlock(u64),
    #[error("Failed to get receipts")]
    FailedToGetReceipts,
    #[error("Failed to get chain id")]
    FailedToGetChainId,
    #[error("Ethereum RPC error {0}")]
    EthError(#[from] alloy::transports::RpcError<TransportErrorKind>),
    #[error("client error {0}")]
    ClientError(#[from] anyhow::Error),
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
        tracing::info!(
            "Getting block {:?}",
            BlockId::Number(BlockNumberOrTag::Number(number))
        );
        let block = self
            .provider
            .get_block(
                BlockId::Number(BlockNumberOrTag::Number(number)),
                true.into(),
            )
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
            .enumerate()
            .map(|(index, rx)| 
                transaction::Receipt::new(rx, BlockItemIdentifier::new(number.into(), index as u64))
            )
            .collect();

        Ok(receipts)
    }

    pub async fn get_transactions(
        &self,
        number: u64,
    ) -> Result<Vec<transaction::Transaction>, Error> {
        let block = self.get_block(number).await?;

        let transactions = if let BlockTransactions::Full(tx) = block.transactions {
            tx
                .into_iter()
                .enumerate()
                .map(|(index, rx)| 
                    transaction::Transaction::new(rx, BlockItemIdentifier::new(number.into(), index as u64))
                )
                .collect()
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

// #[derive(Debug, Error)]
// pub enum Error {
//     #[error("Failed to fetch block {0}")]
//     FailedToFetchBlock(u64),
// }

pub async fn fetch_block_transactions(
    url: &str,
    block_number: u64,
) -> Result<Vec<Transaction>, anyhow::Error> {
    let ws = WsConnect::new(url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;

    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(block_number), true)
        .await?
        .ok_or(anyhow::anyhow!("failed to fetch block {}", block_number))?;
//        .ok_or(Error::FailedToFetchBlock(block_number))?;

    let transactions = if let BlockTransactions::Full(tx) = block.transactions {
        let mut txs = tx
            .into_iter()
            .enumerate()
            .map(|(index, rx)| 
                Transaction::new(rx, BlockItemIdentifier::new(block_number.into(), index as u64))
            )
            .collect::<Vec<_>>();
        txs.sort_by_key(|tx| tx.id().index());
        txs
    } else {
        info!("No full tx");
        vec![]
    };

    Ok(transactions)
}

pub async fn fetch_block_receipts(
    url: &str,
    block_number: u64,
) -> Result<Vec<Receipt>, anyhow::Error> {
    let ws = WsConnect::new(url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;

    let mut receipts = provider
        .get_block_receipts(BlockNumberOrTag::Number(block_number))
        .await?
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, rx)| 
            Receipt::new(rx, BlockItemIdentifier::new(block_number.into(), index as u64))
        )
        .collect::<Vec<_>>();
    
    receipts.sort_by_key(|rx| rx.id().index());

    Ok(receipts)
}
