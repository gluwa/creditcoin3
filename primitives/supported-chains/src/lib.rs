#![cfg_attr(not(feature = "std"), no_std)]

pub mod api;
pub mod chain_removal_listener;
pub mod provider;
use attestor_primitives::ChainEncodingVersion;
use parity_scale_codec::{Decode, DecodeWithMemTracking, Encode};
use precompile_utils::prelude::String;
use scale_info::TypeInfo;
use sp_std::vec::Vec;

#[derive(Debug, Clone)]
pub enum Error {
    NoDelayFoundForStrategy(String), // Will use if/when we add maturity strategies that don't have associated delays
    InvalidFixedDelay(String),
    InvalidStrategy(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, TypeInfo)]
pub struct SupportedChain {
    pub chain_id: u64,
    pub chain_name: Vec<u8>,
    pub chain_encoding: ChainEncodingVersion,
    pub maturity_strategy: String,
}

// Maturity strategy enum used for robustness in attestors
#[derive(Debug, Clone)]
pub enum MaturityStrategy {
    EvmFinalized,
    EvmSafe,
    EvmLatest,
    FixedDelay(u64),
}

// All maturity strategy variants. Using strings to avoid storage migrations.
pub const MATURITY_EVM_FINALIZED: &str = "EvmFinalized";
pub const MATURITY_EVM_SAFE: &str = "EvmSafe";
pub const MATURITY_EVM_LATEST: &str = "EvmLatest";
pub const MATURITY_FIXED_DELAY: &str = "FixedDelay:";

// Most common fixed delay (current attestor default) set here for ease of entry
// in chain_spec, etc.
pub const MATURITY_FIXED_DELAY_10: &str = "FixedDelay: 10";

// Not every maturity strategy will necessarily correspond to a fixed delay in the future.
// So we use option for the possibility of having no corresponding delay for a vaild strategy.
impl MaturityStrategy {
    pub const fn maturity_delay(&self) -> Option<u64> {
        match self {
            Self::EvmFinalized => Some(64),
            Self::EvmSafe => Some(32),
            Self::EvmLatest => Some(0),
            Self::FixedDelay(n) => Some(*n),
        }
    }
}

impl TryFrom<&str> for MaturityStrategy {
    type Error = Error;
    fn try_from(strategy_string: &str) -> Result<Self, Self::Error> {
        match strategy_string {
            MATURITY_EVM_FINALIZED => Ok(MaturityStrategy::EvmFinalized),
            MATURITY_EVM_SAFE => Ok(MaturityStrategy::EvmSafe),
            MATURITY_EVM_LATEST => Ok(MaturityStrategy::EvmLatest),
            _ => {
                if let Some(rest) = strategy_string.strip_prefix(MATURITY_FIXED_DELAY) {
                    let delay = rest
                        .trim()
                        .parse::<u64>()
                        .map_err(|_| Error::InvalidFixedDelay(String::from(strategy_string)))?;

                    Ok(MaturityStrategy::FixedDelay(delay))
                } else {
                    Err(Error::InvalidStrategy(String::from(strategy_string)))
                }
            }
        }
    }
}
