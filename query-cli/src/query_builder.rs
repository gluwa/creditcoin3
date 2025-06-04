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
            Network::Ethereum(_) => BLOCKSCOUT_ETH_URL,
            Network::Sepolia(_) => BLOCKSCOUT_SEPOLIA_URL,
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
        // Path to file of burn ERC20 ABI from bridge-usage-example
        let path = "../bridge-usage-example/TestERC20Abi.txt";
        let abi = std::fs::read_to_string(path).expect("Abi should be there");

        Ok(abi)
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
        Network::Sepolia(_) | Network::Ethereum(_) => {
            let abi_provider = BlockscoutAbiProvider { network };
            query_builder.set_abi_provider(Box::new(abi_provider));
        }
        Network::Local(_) => {
            let abi_provider = PocAbiProvider();
            query_builder.set_abi_provider(Box::new(abi_provider));
        }
        _ => {
            return Err(anyhow!(
                "Unsupported network for ERC20 transfer segments: {:?}",
                network
            ));
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
    let event_builder_result = query_builder
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
        .await;
    if let Err(e) = event_builder_result {
        if let QueryBuilderError::FailedToFindEventByNameOrSignature(_) = e {
            return Err(anyhow!("No ERC20 Transfer found in transaction. Did you provide the correct transaction hash?"));
        } else {
            return Err(anyhow!("Adding event fields failed: {:?}", e));
        }
    }

    let layout_segments = query_builder
        .get_selected_offsets()
        .iter()
        .map(|(offset, size)| LayoutSegment {
            offset: *offset as u64,
            size: *size as u64,
        })
        .collect::<Vec<LayoutSegment>>();

    Ok(layout_segments)
}

/// Gets the layout segments corresponding to relevant fields of a
/// smart contract call which resulted in an ERC20 transfer. Will
/// fail when used on transactions with more than one resulting
/// transfer event.
///
/// Fields:
/// Rx - Status
/// Tx - From
/// Tx - To
/// Tx - Value (Amount)
pub async fn get_native_token_transfer_segments(
    network: Network,
    tx: Transaction,
    rx: TransactionReceipt,
) -> Result<Vec<LayoutSegment>> {
    let mut query_builder = QueryBuilder::create_from_transaction(tx, rx)
        .map_err(|e| anyhow!("Creating query builder failed: {:?}", e))?;

    match network {
        Network::Sepolia(_) | Network::Ethereum(_) => {
            let abi_provider = BlockscoutAbiProvider { network };
            query_builder.set_abi_provider(Box::new(abi_provider));
        }
        Network::Local(_) => {
            let abi_provider = PocAbiProvider();
            query_builder.set_abi_provider(Box::new(abi_provider));
        }
        _ => {
            return Err(anyhow!(
                "Unsupported network for ERC20 transfer segments: {:?}",
                network
            ));
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
    query_builder
        .add_static_field(QueryableFields::TxValue)
        .map_err(|e| anyhow!("Adding value field failed: {:?}", e))?;

    let layout_segments = query_builder
        .get_selected_offsets()
        .iter()
        .map(|(offset, size)| LayoutSegment {
            offset: *offset as u64,
            size: *size as u64,
        })
        .collect::<Vec<LayoutSegment>>();

    Ok(layout_segments)
}
