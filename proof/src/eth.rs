use alloy::providers::Provider;
use alloy::providers::ProviderBuilder;
use alloy::rpc::client::WsConnect;
use alloy::rpc::types::eth::BlockTransactions;
use alloy::rpc::types::eth::{Block, BlockNumberOrTag};
use attestor::transaction::{Receipt, Transaction};
use thiserror::Error;
use tracing::info;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to fetch block {0}")]
    FailedToFetchBlock(u64),
}

pub async fn fetch_block_transactions(
    url: &str,
    block_number: u64,
) -> Result<Vec<Transaction>, anyhow::Error> {
    let ws = WsConnect::new(url);
    let provider = ProviderBuilder::new().on_ws(ws).await?;

    let block = provider
        .get_block_by_number(BlockNumberOrTag::Number(block_number), true)
        .await?
        .ok_or(Error::FailedToFetchBlock(block_number))?;

    let transactions = if let BlockTransactions::Full(tx) = block.transactions {
        tx.into_iter().map(Transaction).collect()
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

    let receipts = provider
        .get_block_receipts(BlockNumberOrTag::Number(block_number))
        .await?;

    let receipts = receipts.into_iter().flatten().map(Receipt).collect();

    Ok(receipts)
}
