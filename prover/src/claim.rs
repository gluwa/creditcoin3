use anyhow::Result;
use cc_client::{cc3::runtime_types::prover_primitives::claim::ClaimKind, claim::Cc3Claim};
use eth::{transaction::BlockItem, Address};

pub async fn check_claim_inclusion(client: eth::Client, claim: Cc3Claim) -> Result<bool> {
    let block = client.get_block(claim.block_number).await?;

    // TODO: find a way to query receipts on a hardhat node (or some sidecar) https://github.com/NomicFoundation/hardhat/issues/4761
    let receipts = client
        .get_receipts(block.header.number.unwrap_or_default())
        .await?;

    // let receipts = receipts.into_iter().flatten().map(eth::Receipt).collect();

    let transactions = client
        .get_transactions(block.header.number.unwrap_or_default())
        .await?;

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
            for receipt in receipts {
                if receipt.0.transaction_index.unwrap_or_default() == u64::from(claim.tx_index)
                    && receipt.0.from == Address::from(claim.from.0)
                    && receipt.0.to == Some(Address::from(claim.to.0))
                {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}
