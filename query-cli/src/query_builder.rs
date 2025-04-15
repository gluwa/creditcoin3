use alloy::rpc::types::{Transaction, TransactionReceipt};
use anyhow::{anyhow, Result};
use ccnext_query_builder::abi::{models::QueryableFields, query_builder::QueryBuilder};
use pallet_prover_primitives::LayoutSegment;
use std::sync::Arc;

/// Gets the layout segments corresponding to relevant fields of a
/// smart contract call which resulted in an ERC20 transfer. Will
/// fail when used on transactions with more than one resulting
/// transfer event.
///
/// Fields:
/// Rx - Status
/// Tx - From
/// Tx - To (contract addr)
/// Event - Addr (contract emitting event)
/// Event - Signature
/// Event - from (address sending ERC20)
/// Event - to (address receiving ERC20)
/// Event - value (sent amount)
pub async fn get_erc20_transfer_segments(
    tx: Transaction,
    rx: TransactionReceipt,
) -> Result<Vec<LayoutSegment>> {
    let mut query_builder = QueryBuilder::create_from_transaction(tx, rx)
        .map_err(|e| anyhow!("Creating query builder failed: {:?}", e))?;
    query_builder.set_abi_provider(Arc::new(|contract_address| {
        Box::pin(sample_abi_provider(contract_address.clone()))
    }));

    query_builder
        .add_static_field(QueryableFields::RxStatus)
        .map_err(|e| anyhow!("Adding status field failed: {:?}", e))?;
    query_builder
        .add_static_field(QueryableFields::TxFrom)
        .map_err(|e| anyhow!("Adding from field failed: {:?}", e))?;
    query_builder
        .add_static_field(QueryableFields::TxTo)
        .map_err(|e| anyhow!("Adding to field failed: {:?}", e))?;

    // Get fields from the transfer event
    query_builder
        .event_builder(
            "Transfer".into(),
            |_log, _event, _log_index| {
                true // No filter applied. We take whatever `Transfer` log is available
            },
            |builder| {
                builder
                    .add_address()?
                    .add_signature()?
                    .add_argument("from")?
                    .add_argument("to")?
                    .add_argument("value")?;
                Ok(())
            },
        )
        .await
        .map_err(|e| anyhow!("Adding event fields failed: {:?}", e))?;

    let layout_segments = query_builder
        .get_selected_offsets()
        .iter()
        .map(|(offset, size)| LayoutSegment {
            offset: *offset as u64,
            size: *size as u64,
        })
        .collect::<Vec<LayoutSegment>>();

    // TODO:: We may need to account for the block item identifier which was being encoded as a part of TxRx.
    // If we can avoid having it encoded along with the transactions and receipts, then that will save us some
    // complexity.
    // But if we do have to encode it. Then we need to account for it by adding its length to the offsets of the
    // layout segments created by our query builder. We would make those adjustments here or in a helper fn.
    
    Ok(layout_segments)
}

// TODO: This is only valid for GCRE contract calls. Dynamically fetch the abi for contract addresses rather than using this default
async fn sample_abi_provider(_contract_address: String) -> Option<String> {
    // hard coded G-CRE's ABI
    let json_str = r#"[{"constant":false,"inputs":[{"name":"tokenHolders","type":"address[]"},{"name":"amounts","type":"uint256[]"}],"name":"recordSales730Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[],"name":"VestingStartDate","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"name","outputs":[{"name":"","type":"string"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"spender","type":"address"},{"name":"value","type":"uint256"}],"name":"approve","outputs":[{"name":"success","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"vestedBalanceOf","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"totalSupply","outputs":[{"name":"amount","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"purchasedBalanceOf365Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"value","type":"uint256"},{"name":"sighash","type":"string"}],"name":"exchange","outputs":[{"name":"success","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"from","type":"address"},{"name":"to","type":"address"},{"name":"value","type":"uint256"}],"name":"transferFrom","outputs":[{"name":"success","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"vestedBalanceOf183Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"decimals","outputs":[{"name":"","type":"uint8"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolders","type":"address[]"},{"name":"amounts","type":"uint256[]"}],"name":"recordSales1095Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"vestedBalanceOf365Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"value","type":"uint256"}],"name":"burn","outputs":[{"name":"success","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"purchasedBalanceOf2190Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolders","type":"address[]"},{"name":"amounts","type":"uint256[]"}],"name":"recordSales183Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolder","type":"address"},{"name":"numCoins","type":"uint256"}],"name":"recordSale365Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"vestedBalanceOf730Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"purchasedBalanceOf","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[{"name":"owner","type":"address"}],"name":"balanceOf","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[],"name":"finalizeSales","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"from","type":"address"},{"name":"value","type":"uint256"}],"name":"burnFrom","outputs":[{"name":"success","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"vestedBalanceOf2190Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"purchasedBalanceOf730Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"symbol","outputs":[{"name":"","type":"string"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolder","type":"address"},{"name":"numCoins","type":"uint256"}],"name":"recordSale183Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"to","type":"address"},{"name":"value","type":"uint256"}],"name":"transfer","outputs":[{"name":"success","type":"bool"}],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[],"name":"creditcoinSalesLimit","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"vestedBalanceOf1095Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"creditcoinLimitInFrac","outputs":[{"name":"","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolder","type":"address"},{"name":"numCoins","type":"uint256"}],"name":"recordSale2190Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolder","type":"address"},{"name":"numCoins","type":"uint256"}],"name":"recordSale730Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"purchasedBalanceOf1095Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[{"name":"owner","type":"address"},{"name":"spender","type":"address"}],"name":"allowance","outputs":[{"name":"remaining","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[],"name":"startVesting","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolders","type":"address[]"},{"name":"amounts","type":"uint256[]"}],"name":"recordSales2190Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":true,"inputs":[{"name":"tokenHolder","type":"address"}],"name":"purchasedBalanceOf183Days","outputs":[{"name":"balance","type":"uint256"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":true,"inputs":[],"name":"IsSalesFinalized","outputs":[{"name":"","type":"bool"}],"payable":false,"stateMutability":"view","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolders","type":"address[]"},{"name":"amounts","type":"uint256[]"}],"name":"recordSales365Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"constant":false,"inputs":[{"name":"tokenHolder","type":"address"},{"name":"numCoins","type":"uint256"}],"name":"recordSale1095Days","outputs":[],"payable":false,"stateMutability":"nonpayable","type":"function"},{"inputs":[{"name":"creditcoinFoundation","type":"address"},{"name":"devCost","type":"address"}],"payable":false,"stateMutability":"nonpayable","type":"constructor"},{"payable":true,"stateMutability":"payable","type":"fallback"},{"anonymous":false,"inputs":[{"indexed":true,"name":"from","type":"address"},{"indexed":false,"name":"value","type":"uint256"},{"indexed":true,"name":"sighash","type":"string"}],"name":"Exchange","type":"event"},{"anonymous":false,"inputs":[{"indexed":true,"name":"from","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Burnt","type":"event"},{"anonymous":false,"inputs":[{"indexed":true,"name":"from","type":"address"},{"indexed":true,"name":"to","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Transfer","type":"event"},{"anonymous":false,"inputs":[{"indexed":true,"name":"owner","type":"address"},{"indexed":true,"name":"spender","type":"address"},{"indexed":false,"name":"value","type":"uint256"}],"name":"Approval","type":"event"}]"#;

    Some(json_str.into())
}
