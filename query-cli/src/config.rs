//! Configuration module
//!
//! This module handles configuration for the query-cli tool,
//! including network settings and query parameters.

use attestor_primitives::{LayoutSegment, Query};
use serde::{Deserialize, Serialize};

/// Network configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Network {
    Sepolia,
    Ethereum,
    Local(String),
    Custom { id: u64, url: String },
}

impl Network {
    /// Get the chain ID for the network
    pub fn chain_id(&self) -> u64 {
        match self {
            Network::Sepolia => 11155111,
            Network::Ethereum => 1,
            Network::Local(_) => 2, // Default for local networks
            Network::Custom { id, .. } => *id,
        }
    }

    /// Get the RPC URL for the network
    pub fn rpc_url(&self) -> String {
        match self {
            Network::Sepolia => "wss://sepolia.infura.io/ws/v3/YOUR-API-KEY".to_string(),
            Network::Ethereum => "wss://mainnet.infura.io/ws/v3/YOUR-API-KEY".to_string(),
            Network::Local(url) => url.clone(),
            Network::Custom { url, .. } => url.clone(),
        }
    }
}

/// Data selection for queries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DataSelection {
    AllData,
    Range { offset: usize, size: usize },
    ERC20Transfer,
    NativeTokenTransfer,
}

impl DataSelection {
    /// Convert to layout segments based on the data selection
    pub fn to_layout_segments(&self) -> Vec<LayoutSegment> {
        match self {
            DataSelection::AllData => vec![LayoutSegment {
                offset: 0,
                size: 99326, // Default max size
            }],
            DataSelection::Range { offset, size } => vec![LayoutSegment {
                offset: *offset as u64,
                size: *size as u64,
            }],
            DataSelection::ERC20Transfer => {
                // Standard ERC20 Transfer event data locations
                vec![
                    LayoutSegment {
                        offset: 0,
                        size: 32, // Event signature
                    },
                    LayoutSegment {
                        offset: 32,
                        size: 32, // From address
                    },
                    LayoutSegment {
                        offset: 64,
                        size: 32, // To address
                    },
                    LayoutSegment {
                        offset: 96,
                        size: 32, // Amount
                    },
                ]
            }
            DataSelection::NativeTokenTransfer => {
                // Standard locations for native token transfers
                vec![
                    LayoutSegment {
                        offset: 479,
                        size: 32, // Nonce
                    },
                    LayoutSegment {
                        offset: 223,
                        size: 32, // From address
                    },
                    LayoutSegment {
                        offset: 255,
                        size: 32, // To address
                    },
                    LayoutSegment {
                        offset: 287,
                        size: 32, // Value
                    },
                ]
            }
        }
    }
}

/// Query configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConfig {
    pub network: Network,
    pub block_height: u64,
    pub transaction_hash: String,
    pub data_selection: DataSelection,
}

impl QueryConfig {
    /// Create a Query from the configuration and transaction index
    pub fn to_query(&self, _tx_index: u64) -> Query {
        Query {
            chain_id: self.network.chain_id(),
            height: self.block_height,
            layout_segments: self.data_selection.to_layout_segments(),
        }
    }
}

/// Creditcoin3 configuration
#[derive(Debug, Clone)]
pub struct CreditcoinConfig {
    pub rpc_url: String,
    pub evm_private_key: String,
}

impl CreditcoinConfig {
    /// Create a new Creditcoin3 configuration
    pub fn new(rpc_url: String, evm_private_key: String) -> Self {
        Self {
            rpc_url,
            evm_private_key,
        }
    }
}

/// Application configuration
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub query: QueryConfig,
    pub creditcoin: CreditcoinConfig,
}

impl AppConfig {
    /// Create a new application configuration
    pub fn new(query: QueryConfig, creditcoin: CreditcoinConfig) -> Self {
        Self { query, creditcoin }
    }
}
