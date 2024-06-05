use alloy::{
    core::primitives::Address,
    providers::{Provider, ProviderBuilder, RootProvider},
    pubsub::PubSubFrontend,
    rpc::{
        client::WsConnect,
        types::eth::{Block, BlockId, BlockNumberOrTag, BlockTransactions},
    },
};
use anyhow::Result;
use cc_client::{cc3::runtime_types::prover_primitives::claim::ClaimKind, claim::Cc3Claim};
use thiserror::Error;
use tracing::{error, info};

use crate::transaction::BlockItem;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get block {0}")]
    FailedToGetBlock(u64),
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

    pub async fn get_block(&self, number: u64) -> Result<Block> {
        let block = self
            .provider
            .get_block(BlockId::Number(BlockNumberOrTag::Number(number)), true)
            .await?;

        if let Some(block) = block {
            Ok(block)
        } else {
            Err(Error::FailedToGetBlock(number).into())
        }
    }

    pub async fn check_claim_inclusion(&self, claim: Cc3Claim) -> Result<bool> {
        let block = self.get_block(claim.block_number).await?;

        // TODO: find a way to query receipts on a hardhat node (or some sidecar) https://github.com/NomicFoundation/hardhat/issues/4761
        let receipts = self
            .provider
            .get_block_receipts(alloy::rpc::types::eth::BlockNumberOrTag::Number(
                block.header.number.unwrap(),
            ))
            .await?;

        // let receipts = receipts.into_iter().flatten().map(eth::Receipt).collect();

        let transactions = if let BlockTransactions::Full(tx) = block.transactions {
            tx.into_iter()
                .map(super::transaction::Transaction)
                .collect()
        } else {
            info!("No full tx");
            vec![]
        };

        match claim.kind {
            ClaimKind::Tx => {
                // Check if the claim is included in any of the transactions
                for tx in transactions {
                    if tx.0.transaction_index.unwrap_or_default() == u64::from(claim.tx_index)
                        && tx.from() == Address::from(claim.from.0)
                        && tx.to() == Some(Address::from(claim.to.0))
                    {
                        return Ok(true);
                    }
                }
            }
            ClaimKind::Rx => {
                // Check if the claim is included in any of the receipts
                for receipt in receipts.into_iter().flatten() {
                    if receipt.transaction_index.unwrap_or_default() == u64::from(claim.tx_index)
                        && receipt.from == Address::from(claim.from.0)
                        && receipt.to == Some(Address::from(claim.to.0))
                    {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }
}
