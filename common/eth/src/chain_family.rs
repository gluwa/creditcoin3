//! Source-chain **family**: which execution-layer dialect a source chain speaks.
//!
//! Every chain the attestor can follow exposes the standard Ethereum JSON-RPC surface and a
//! standard RLP block header, so header hashing and the continuity proof are family-agnostic.
//! What differs between families is the *body*: the set of EIP-2718 transaction types a block may
//! contain, how those transactions and their receipts are RLP-encoded for the header's
//! `transactionsRoot` / `receiptsRoot`, and how the attestor turns each (tx, receipt) pair into a
//! Merkle leaf.
//!
//! - [`ChainFamily::Ethereum`]: L1 and L1-shaped chains (Sepolia, Anvil, Polygon PoS). Only
//!   transaction types `0x0`–`0x4` appear.
//! - [`ChainFamily::OpStack`]: OP-Stack rollups (Base, OP Mainnet and their Sepolia testnets).
//!   Every block opens with a type `0x7e` **deposit** transaction carrying the L1 attributes, and
//!   deposit receipts carry extra RLP fields. See [`crate::op_stack`].
//!
//! The family is an **off-chain** setting: it changes how an attestor *reads* a block, not what
//! it attests to. Two attestors that disagree on the family for a chain would compute different
//! leaves and simply fail to reach quorum with each other, so misconfiguration is loud, not
//! unsafe. The default is always [`ChainFamily::Ethereum`], regardless of the RPC's
//! `eth_chainId`. Every OP-Stack chain requires explicit `op-stack` configuration.

use std::fmt;
use std::str::FromStr;

/// Execution-layer dialect of a source chain. See the [module docs](self).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ChainFamily {
    /// Plain Ethereum execution layer: transaction types `0x0`–`0x4` only.
    #[default]
    Ethereum,
    /// OP-Stack rollup: Ethereum types plus the `0x7e` deposit transaction.
    OpStack,
}

/// OP Mainnet.
pub const OP_MAINNET_CHAIN_ID: u64 = 10;
/// OP Sepolia.
pub const OP_SEPOLIA_CHAIN_ID: u64 = 11_155_420;
/// Base mainnet.
pub const BASE_MAINNET_CHAIN_ID: u64 = 8453;
/// Base Sepolia.
pub const BASE_SEPOLIA_CHAIN_ID: u64 = 84_532;

impl ChainFamily {
    /// Whether this family admits the given EIP-2718 transaction type byte.
    pub fn supports_tx_type(self, ty: u8) -> bool {
        match self {
            Self::Ethereum => ty <= 4,
            Self::OpStack => ty <= 4 || ty == crate::op_stack::DEPOSIT_TX_TYPE,
        }
    }

    /// Canonical CLI / config spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ethereum => "ethereum",
            Self::OpStack => "op-stack",
        }
    }
}

impl fmt::Display for ChainFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a chain-family string is not recognised.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown chain family `{0}` (expected `ethereum` or `op-stack`)")]
pub struct ParseChainFamilyError(pub String);

impl FromStr for ChainFamily {
    type Err = ParseChainFamilyError;

    /// Accepts the canonical spellings plus a few aliases (`eth`, `opstack`, `optimism`,
    /// `base`), case-insensitively.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ethereum" | "eth" | "l1" => Ok(Self::Ethereum),
            "op-stack" | "opstack" | "op_stack" | "optimism" | "op" | "base" => Ok(Self::OpStack),
            other => Err(ParseChainFamilyError(other.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_ethereum() {
        assert_eq!(ChainFamily::default(), ChainFamily::Ethereum);
    }

    #[test]
    fn tx_type_support_per_family() {
        for ty in 0u8..=4 {
            assert!(ChainFamily::Ethereum.supports_tx_type(ty));
            assert!(ChainFamily::OpStack.supports_tx_type(ty));
        }
        assert!(!ChainFamily::Ethereum.supports_tx_type(0x7e));
        assert!(ChainFamily::OpStack.supports_tx_type(0x7e));
        assert!(!ChainFamily::OpStack.supports_tx_type(0x6a));
    }

    #[test]
    fn parses_spellings_and_round_trips_display() {
        assert_eq!(
            "ethereum".parse::<ChainFamily>().unwrap(),
            ChainFamily::Ethereum
        );
        assert_eq!(
            "op-stack".parse::<ChainFamily>().unwrap(),
            ChainFamily::OpStack
        );
        assert_eq!(
            "OpStack".parse::<ChainFamily>().unwrap(),
            ChainFamily::OpStack
        );
        assert_eq!("base".parse::<ChainFamily>().unwrap(), ChainFamily::OpStack);
        assert!("nitro".parse::<ChainFamily>().is_err());
        for family in [ChainFamily::Ethereum, ChainFamily::OpStack] {
            assert_eq!(family.to_string().parse::<ChainFamily>().unwrap(), family);
        }
    }

    #[test]
    fn serde_uses_kebab_case() {
        assert_eq!(
            serde_json::to_string(&ChainFamily::OpStack).unwrap(),
            "\"op-stack\""
        );
        assert_eq!(
            serde_json::from_str::<ChainFamily>("\"ethereum\"").unwrap(),
            ChainFamily::Ethereum
        );
    }
}
