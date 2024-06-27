use serde::Deserialize;

use attestor_primitives::ChainId;
use cc_client::ChainPriceConfig;

#[derive(Debug, Clone)]
/// Server configuration
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
    pub postgres_uri: String,
}

impl Config {
    #[must_use]
    pub fn get_chains(&self) -> Vec<u64> {
        self.chain_price_configurations
            .chain
            .iter()
            .map(|chain| chain.chain_id)
            .collect()
    }
}

impl ChainPriceConfigurations {
    #[must_use]
    pub fn get_rpc_url(&self, chain_id: ChainId) -> Option<String> {
        self.chain
            .iter()
            .find(|chain| chain.chain_id == chain_id)
            .map(|chain| chain.rpc_url.clone())
    }
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

impl Into<ChainPriceConfig> for Chain {
    fn into(self) -> ChainPriceConfig {
        ChainPriceConfig {
            chain_id: self.chain_id,
            price: self.price,
        }
    }
}
