use anyhow::Result;
use async_trait::async_trait;

use ccnext_query_builder::abi::{models::QueryBuilderError, query_builder::AbiProvider};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::Network;

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
