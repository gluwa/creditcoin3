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

/// How far behind the source chain tip an attestor (and the proof server) considers a block
/// mature enough to attest to.
///
/// Two families of strategy exist:
///
/// - **Fixed offsets** ([`EvmFinalized`](Self::EvmFinalized), [`EvmSafe`](Self::EvmSafe),
///   [`EvmLatest`](Self::EvmLatest), [`FixedDelay`](Self::FixedDelay)): the mature height is
///   `tip - delay`. `EvmSafe` / `EvmFinalized` are the Ethereum-epoch approximations (32 / 64
///   blocks); they are *not* RPC tags and mean very different things on a 2 s rollup.
/// - **RPC tags** ([`RpcSafe`](Self::RpcSafe), [`RpcFinalized`](Self::RpcFinalized)): the mature
///   height is whatever the source node reports for `eth_getBlockByNumber("safe" | "finalized")`.
///   On Ethereum that tracks justification / finality; on OP-Stack and Arbitrum rollups it tracks
///   L1 batch inclusion / L1 finality, which a fixed block count cannot follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaturityStrategy {
    EvmFinalized,
    EvmSafe,
    EvmLatest,
    FixedDelay(u64),
    /// Mature height = the node's `safe` block.
    RpcSafe,
    /// Mature height = the node's `finalized` block.
    RpcFinalized,
}

/// A source-chain block tag a [`MaturityStrategy`] may resolve through the RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcBlockTag {
    Safe,
    Finalized,
}

impl RpcBlockTag {
    /// The JSON-RPC block tag string.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Finalized => "finalized",
        }
    }
}

// All maturity strategy variants. Using strings to avoid storage migrations.
pub const MATURITY_EVM_FINALIZED: &str = "EvmFinalized";
pub const MATURITY_EVM_SAFE: &str = "EvmSafe";
pub const MATURITY_EVM_LATEST: &str = "EvmLatest";
pub const MATURITY_FIXED_DELAY: &str = "FixedDelay:";
pub const MATURITY_RPC_SAFE: &str = "RpcSafe";
pub const MATURITY_RPC_FINALIZED: &str = "RpcFinalized";

// Most common fixed delay (current attestor default) set here for ease of entry
// in chain_spec, etc.
pub const MATURITY_FIXED_DELAY_10: &str = "FixedDelay: 10";

// Not every maturity strategy will necessarily correspond to a fixed delay in the future.
// So we use option for the possibility of having no corresponding delay for a vaild strategy.
impl MaturityStrategy {
    /// Fixed block offset behind the tip, for the offset-based strategies. `None` for the RPC-tag
    /// strategies, whose offset varies block to block; use [`rpc_tag`](Self::rpc_tag) for those.
    pub const fn maturity_delay(&self) -> Option<u64> {
        match self {
            Self::EvmFinalized => Some(64),
            Self::EvmSafe => Some(32),
            Self::EvmLatest => Some(0),
            Self::FixedDelay(n) => Some(*n),
            Self::RpcSafe | Self::RpcFinalized => None,
        }
    }

    /// The RPC block tag to resolve maturity through, for the tag-based strategies.
    pub const fn rpc_tag(&self) -> Option<RpcBlockTag> {
        match self {
            Self::RpcSafe => Some(RpcBlockTag::Safe),
            Self::RpcFinalized => Some(RpcBlockTag::Finalized),
            Self::EvmFinalized | Self::EvmSafe | Self::EvmLatest | Self::FixedDelay(_) => None,
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
            MATURITY_RPC_SAFE => Ok(MaturityStrategy::RpcSafe),
            MATURITY_RPC_FINALIZED => Ok(MaturityStrategy::RpcFinalized),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_strategy_string() {
        assert_eq!(
            MaturityStrategy::try_from(MATURITY_RPC_SAFE).unwrap(),
            MaturityStrategy::RpcSafe
        );
        assert_eq!(
            MaturityStrategy::try_from(MATURITY_RPC_FINALIZED).unwrap(),
            MaturityStrategy::RpcFinalized
        );
        assert_eq!(
            MaturityStrategy::try_from(MATURITY_EVM_SAFE).unwrap(),
            MaturityStrategy::EvmSafe
        );
        assert_eq!(
            MaturityStrategy::try_from("FixedDelay: 20").unwrap(),
            MaturityStrategy::FixedDelay(20)
        );
        assert!(MaturityStrategy::try_from("rpcsafe").is_err());
        assert!(MaturityStrategy::try_from("RpcLatest").is_err());
    }

    #[test]
    fn rpc_strategies_have_a_tag_and_no_fixed_delay() {
        for (strategy, tag) in [
            (MaturityStrategy::RpcSafe, RpcBlockTag::Safe),
            (MaturityStrategy::RpcFinalized, RpcBlockTag::Finalized),
        ] {
            assert_eq!(strategy.maturity_delay(), None);
            assert_eq!(strategy.rpc_tag(), Some(tag));
        }
        for strategy in [
            MaturityStrategy::EvmFinalized,
            MaturityStrategy::EvmSafe,
            MaturityStrategy::EvmLatest,
            MaturityStrategy::FixedDelay(7),
        ] {
            assert!(strategy.maturity_delay().is_some());
            assert_eq!(strategy.rpc_tag(), None);
        }
        assert_eq!(RpcBlockTag::Safe.as_str(), "safe");
        assert_eq!(RpcBlockTag::Finalized.as_str(), "finalized");
    }
}
