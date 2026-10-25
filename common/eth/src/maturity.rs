//! Block **maturity**: how far behind the source chain tip a block must be before the attestor
//! treats it as settled.
//!
//! Two flavours exist, mirroring the on-chain `MaturityStrategy` strings in the supported-chains
//! pallet:
//!
//! - [`Maturity::FixedLag`]: the mature height is `head - lag`. Cheap, deterministic, and blind
//!   to what the chain itself considers safe.
//! - [`Maturity::Tag`]: the mature height is whatever the node reports for the `safe` or
//!   `finalized` block tag. On Ethereum that follows justification and finality; on OP-Stack and
//!   Arbitrum rollups it follows L1 batch inclusion and L1 finality, which no fixed block count
//!   can track (a stalled batcher can hold `safe` back for hours).
//!
//! Both are resolved through [`Maturity::mature_height`], which the tip and roots streams call
//! once per new source head.

use crate::{Client, Error};
use alloy::rpc::types::BlockNumberOrTag;
use std::fmt;

/// A source-chain block tag with settlement meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockTag {
    /// `safe`: justified on Ethereum; posted to L1 on rollups.
    Safe,
    /// `finalized`: finalized on Ethereum; L1-finalized on rollups.
    Finalized,
}

impl BlockTag {
    /// The JSON-RPC spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Finalized => "finalized",
        }
    }
}

impl From<BlockTag> for BlockNumberOrTag {
    fn from(tag: BlockTag) -> Self {
        match tag {
            BlockTag::Safe => BlockNumberOrTag::Safe,
            BlockTag::Finalized => BlockNumberOrTag::Finalized,
        }
    }
}

impl fmt::Display for BlockTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the attestor decides which source height is mature. See the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Maturity {
    /// Mature height is `head - lag`.
    FixedLag(u64),
    /// Mature height is the node's block for `tag`, clamped to the observed head.
    Tag(BlockTag),
}

impl Maturity {
    /// Whether resolving maturity needs an RPC round-trip per head.
    pub const fn needs_rpc(self) -> bool {
        matches!(self, Self::Tag(_))
    }

    /// The mature height for source head `head`, or `None` when nothing is mature yet (a fixed
    /// lag larger than the head).
    ///
    /// For [`Maturity::Tag`] this asks the node for the tagged block and clamps the answer to
    /// `head`: a load-balanced endpoint can report a `safe` block the subscription has not
    /// delivered yet, and the streams must never run ahead of the heads they have seen. A node
    /// that does not serve the tag surfaces as [`Error::FailedToGetBlockByTag`]; callers should
    /// log and retry on the next head rather than guess.
    pub async fn mature_height(self, client: &Client, head: u64) -> Result<Option<u64>, Error> {
        match self {
            Self::FixedLag(lag) => Ok(head.checked_sub(lag)),
            Self::Tag(tag) => {
                let tagged = client.get_block_number_by_tag(tag).await?;
                Ok(Some(tagged.min(head)))
            }
        }
    }

    /// Pure helper for the roots stream: the block numbers that became mature when the mature
    /// height moved from `next_unfetched - 1` to `mature`. Empty when nothing new matured.
    pub fn newly_mature(next_unfetched: u64, mature: u64) -> std::ops::RangeInclusive<u64> {
        if mature >= next_unfetched {
            next_unfetched..=mature
        } else {
            #[allow(clippy::reversed_empty_ranges)]
            {
                1..=0
            }
        }
    }
}

impl fmt::Display for Maturity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FixedLag(lag) => write!(f, "fixed lag of {lag} blocks"),
            Self::Tag(tag) => write!(f, "`{tag}` block tag"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newly_mature_ranges() {
        assert_eq!(
            Maturity::newly_mature(10, 12).collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
        assert_eq!(Maturity::newly_mature(10, 10).collect::<Vec<_>>(), vec![10]);
        assert!(Maturity::newly_mature(10, 9).next().is_none());
        assert!(Maturity::newly_mature(10, 0).next().is_none());
    }

    #[test]
    fn tags_map_to_alloy_and_strings() {
        assert_eq!(
            BlockNumberOrTag::from(BlockTag::Safe),
            BlockNumberOrTag::Safe
        );
        assert_eq!(
            BlockNumberOrTag::from(BlockTag::Finalized),
            BlockNumberOrTag::Finalized
        );
        assert_eq!(BlockTag::Safe.to_string(), "safe");
        assert_eq!(
            Maturity::Tag(BlockTag::Finalized).to_string(),
            "`finalized` block tag"
        );
        assert_eq!(Maturity::FixedLag(32).to_string(), "fixed lag of 32 blocks");
        assert!(Maturity::Tag(BlockTag::Safe).needs_rpc());
        assert!(!Maturity::FixedLag(0).needs_rpc());
    }
}
