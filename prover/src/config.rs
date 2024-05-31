use serde::Deserialize;

use attestor_primitives::ChainId;
use cc_client::cc3::runtime_types::prover_primitives::ChainPriceConfiguration as RuntimeChainPriceConfiguration;

#[derive(Debug, Clone)]
/// Server configuration
/// - `eth_rpc_url`: Source chain RPC url
/// - `cc3_rpc_url`: Creditcoin RPC url (must have rpc + websocket features)
/// - `cc3_key`: Mnemonic for a creditcoin3 account
/// - `nickname`: Nickname for this prover
/// - `claim_buffer`: The amount of claims we can handle in a certain period
/// - `chain_price_configurations`: A list of chains with their configured price
pub struct Config {
    pub cc3_rpc_url: String,
    pub cc3_key: String,
    pub nickname: String,
    pub claim_buffer: u8,
    pub chain_price_configurations: ChainPriceConfigurations,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChainPriceConfigurations {
    pub chain: Vec<Chain>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Chain {
    pub rpc_url: String,
    pub chain_id: ChainId,
    pub price: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainPriceConfiguration {
    pub chain_id: ChainId,
    pub price: u64,
}

impl From<RuntimeChainPriceConfiguration> for ChainPriceConfiguration {
    fn from(config: RuntimeChainPriceConfiguration) -> Self {
        ChainPriceConfiguration {
            chain_id: config.chain_id,
            price: config.price,
        }
    }
}

impl Into<RuntimeChainPriceConfiguration> for ChainPriceConfiguration {
    fn into(self) -> RuntimeChainPriceConfiguration {
        RuntimeChainPriceConfiguration {
            chain_id: self.chain_id,
            price: self.price,
        }
    }
}

impl Into<ChainPriceConfiguration> for Chain {
    fn into(self) -> ChainPriceConfiguration {
        ChainPriceConfiguration {
            chain_id: self.chain_id,
            price: self.price,
        }
    }
}
