use alloy::providers::Provider;
use alloy::providers::ProviderBuilder;
use alloy::rpc::client::WsConnect;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::rpc::types::eth::BlockTransactions;
use eth_common::transaction::{Receipt, Transaction, BlockItem};
use thiserror::Error;
use tracing::info;
use utils::block_item_traits::BlockItemIdentifier;

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
