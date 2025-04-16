use super::Network;
use alloy::rpc::types::{Transaction, TransactionReceipt};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use ccnext_query_builder::abi::{
    models::{QueryBuilderError, QueryableFields},
    query_builder::{AbiProvider, QueryBuilder},
};
use pallet_prover_primitives::LayoutSegment;
use reqwest::Client;
use serde::{Deserialize, Serialize};

// Header names and url expected by etherscan
const MODULE: &str = "module";
const ACTION: &str = "action";
const ADDRESS: &str = "address";
const BLOCKSCOUT_ETH_URL: &str = "https://eth.blockscout.com/api";
const BLOCKSCOUT_SEPOLIA_URL: &str = "https://eth-sepolia.blockscout.com/api";

pub struct BlockscoutAbiProvider {
    pub network: Network,
}

#[derive(Serialize, Deserialize, Debug)]
struct AbiResponse {
    message: String,
    result: Option<String>,
    status: String,
}

#[async_trait]
impl AbiProvider for BlockscoutAbiProvider {
    async fn get_abi(&self, contract_address: String) -> Result<String, QueryBuilderError> {
        let client = Client::new();
        let url = match self.network {
            Network::Ethereum => BLOCKSCOUT_ETH_URL,
            Network::Sepolia => BLOCKSCOUT_SEPOLIA_URL,
            _ => {
                return Err(QueryBuilderError::ContractAbiRetrievalFailed {
                    contract_addr: contract_address.clone(),
                    error_message: "Tried to use blockscout to get local network contract."
                        .to_string(),
                });
            }
        };

        let params = [
            (MODULE, "contract"),
            (ACTION, "getabi"),
            (ADDRESS, &contract_address),
        ];

        let response = client.get(url).query(&params).send().await.map_err(|e| {
            QueryBuilderError::ContractAbiRetrievalFailed {
                contract_addr: contract_address.clone(),
                error_message: e.to_string(),
            }
        })?;

        let response = match response.status() {
            reqwest::StatusCode::OK => response.json::<AbiResponse>().await.map_err(|e| {
                QueryBuilderError::ContractAbiRetrievalFailed {
                    contract_addr: contract_address.clone(),
                    error_message: e.to_string(),
                }
            })?,
            other => {
                return Err(QueryBuilderError::ContractAbiRetrievalFailed {
                    contract_addr: contract_address.clone(),
                    error_message: format!("Bad response status: {:?}", other.as_str()),
                });
            }
        };

        if let Some(abi) = response.result {
            Ok(abi)
        } else {
            return Err(QueryBuilderError::ContractAbiRetrievalFailed {
                contract_addr: contract_address,
                error_message: format!(
                    "Response result field empty. Response message: {:?}, Response Status: {:?}",
                    response.message, response.status
                ),
            });
        }
    }
}

pub struct PocAbiProvider();

#[async_trait]
impl AbiProvider for PocAbiProvider {
    async fn get_abi(&self, _contract_address: String) -> Result<String, QueryBuilderError> {
        // TODO: Test if this hardcoded ABI works
        // Hard coded burn ERC20 ABI from bridge-usage-example
        let json_str = "[{\"inputs\":[],\"stateMutability\":\"nonpayable\",\"type\":\"constructor\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"spender\",\"type\":\"address\"},{\"internalType\":\"uint256\",\"name\":\"allowance\",\"type\":\"uint256\"},{\"internalType\":\"uint256\",\"name\":\"needed\",\"type\":\"uint256\"}],\"name\":\"ERC20InsufficientAllowance\",\"type\":\"error\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"sender\",\"type\":\"address\"},{\"internalType\":\"uint256\",\"name\":\"balance\",\"type\":\"uint256\"},{\"internalType\":\"uint256\",\"name\":\"needed\",\"type\":\"uint256\"}],\"name\":\"ERC20InsufficientBalance\",\"type\":\"error\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"approver\",\"type\":\"address\"}],\"name\":\"ERC20InvalidApprover\",\"type\":\"error\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"receiver\",\"type\":\"address\"}],\"name\":\"ERC20InvalidReceiver\",\"type\":\"error\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"sender\",\"type\":\"address\"}],\"name\":\"ERC20InvalidSender\",\"type\":\"error\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"spender\",\"type\":\"address\"}],\"name\":\"ERC20InvalidSpender\",\"type\":\"error\"},{\"anonymous\":false,\"inputs\":[{\"indexed\":true,\"internalType\":\"address\",\"name\":\"owner\",\"type\":\"address\"},{\"indexed\":true,\"internalType\":\"address\",\"name\":\"spender\",\"type\":\"address\"},{\"indexed\":false,\"internalType\":\"uint256\",\"name\":\"value\",\"type\":\"uint256\"}],\"name\":\"Approval\",\"type\":\"event\"},{\"anonymous\":false,\"inputs\":[{\"indexed\":true,\"internalType\":\"address\",\"name\":\"from\",\"type\":\"address\"},{\"indexed\":true,\"internalType\":\"address\",\"name\":\"to\",\"type\":\"address\"},{\"indexed\":false,\"internalType\":\"uint256\",\"name\":\"value\",\"type\":\"uint256\"}],\"name\":\"Transfer\",\"type\":\"event\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"owner\",\"type\":\"address\"},{\"internalType\":\"address\",\"name\":\"spender\",\"type\":\"address\"}],\"name\":\"allowance\",\"outputs\":[{\"internalType\":\"uint256\",\"name\":\"\",\"type\":\"uint256\"}],\"stateMutability\":\"view\",\"type\":\"function\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"spender\",\"type\":\"address\"},{\"internalType\":\"uint256\",\"name\":\"value\",\"type\":\"uint256\"}],\"name\":\"approve\",\"outputs\":[{\"internalType\":\"bool\",\"name\":\"\",\"type\":\"bool\"}],\"stateMutability\":\"nonpayable\",\"type\":\"function\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"account\",\"type\":\"address\"}],\"name\":\"balanceOf\",\"outputs\":[{\"internalType\":\"uint256\",\"name\":\"\",\"type\":\"uint256\"}],\"stateMutability\":\"view\",\"type\":\"function\"},{\"inputs\":[],\"name\":\"decimals\",\"outputs\":[{\"internalType\":\"uint8\",\"name\":\"\",\"type\":\"uint8\"}],\"stateMutability\":\"view\",\"type\":\"function\"},{\"inputs\":[],\"name\":\"name\",\"outputs\":[{\"internalType\":\"string\",\"name\":\"\",\"type\":\"string\"}],\"stateMutability\":\"view\",\"type\":\"function\"},{\"inputs\":[],\"name\":\"symbol\",\"outputs\":[{\"internalType\":\"string\",\"name\":\"\",\"type\":\"string\"}],\"stateMutability\":\"view\",\"type\":\"function\"},{\"inputs\":[],\"name\":\"totalSupply\",\"outputs\":[{\"internalType\":\"uint256\",\"name\":\"\",\"type\":\"uint256\"}],\"stateMutability\":\"view\",\"type\":\"function\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"to\",\"type\":\"address\"},{\"internalType\":\"uint256\",\"name\":\"value\",\"type\":\"uint256\"}],\"name\":\"transfer\",\"outputs\":[{\"internalType\":\"bool\",\"name\":\"\",\"type\":\"bool\"}],\"stateMutability\":\"nonpayable\",\"type\":\"function\"},{\"inputs\":[{\"internalType\":\"address\",\"name\":\"from\",\"type\":\"address\"},{\"internalType\":\"address\",\"name\":\"to\",\"type\":\"address\"},{\"internalType\":\"uint256\",\"name\":\"value\",\"type\":\"uint256\"}],\"name\":\"transferFrom\",\"outputs\":[{\"internalType\":\"bool\",\"name\":\"\",\"type\":\"bool\"}],\"stateMutability\":\"nonpayable\",\"type\":\"function\"}]\n";

        Ok(json_str.into())
    }
}

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
    network: Network,
    tx: Transaction,
    rx: TransactionReceipt,
) -> Result<Vec<LayoutSegment>> {
    let mut query_builder = QueryBuilder::create_from_transaction(tx, rx)
        .map_err(|e| anyhow!("Creating query builder failed: {:?}", e))?;

    match network {
        Network::Sepolia | Network::Ethereum => {
            let abi_provider = BlockscoutAbiProvider { network };
            query_builder.set_abi_provider(Box::new(abi_provider));
        }
        Network::Local => {
            let abi_provider = PocAbiProvider();
            query_builder.set_abi_provider(Box::new(abi_provider));
        }
    }

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
