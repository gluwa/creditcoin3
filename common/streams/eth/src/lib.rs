mod error;
pub mod roots;
pub mod tip;

pub use error::Error;
pub use roots::StreamRoots;
pub use tip::StreamTip;

/// Resolve the on-chain `MaturityStrategy` of a supported chain into the off-chain
/// [`eth::Maturity`] the streams act on.
///
/// Offset strategies (`EvmSafe`, `EvmFinalized`, `EvmLatest`, `FixedDelay: n`) become a fixed
/// lag; the RPC-tag strategies (`RpcSafe`, `RpcFinalized`) become a block-tag lookup. Returns
/// `None` only for a strategy that has neither, which no current variant is — kept as an
/// `Option` so a future strategy cannot be silently mis-resolved as "no lag".
pub fn maturity_from_strategy(
    strategy: &supported_chains_primitives::MaturityStrategy,
) -> Option<eth::Maturity> {
    use supported_chains_primitives::RpcBlockTag;
    if let Some(lag) = strategy.maturity_delay() {
        return Some(eth::Maturity::FixedLag(lag));
    }
    strategy.rpc_tag().map(|tag| match tag {
        RpcBlockTag::Safe => eth::Maturity::Tag(eth::BlockTag::Safe),
        RpcBlockTag::Finalized => eth::Maturity::Tag(eth::BlockTag::Finalized),
    })
}

#[cfg(test)]
mod maturity_tests {
    use super::*;
    use supported_chains_primitives::MaturityStrategy;

    #[test]
    fn every_strategy_resolves() {
        assert_eq!(
            maturity_from_strategy(&MaturityStrategy::EvmSafe),
            Some(eth::Maturity::FixedLag(32))
        );
        assert_eq!(
            maturity_from_strategy(&MaturityStrategy::FixedDelay(20)),
            Some(eth::Maturity::FixedLag(20))
        );
        assert_eq!(
            maturity_from_strategy(&MaturityStrategy::RpcSafe),
            Some(eth::Maturity::Tag(eth::BlockTag::Safe))
        );
        assert_eq!(
            maturity_from_strategy(&MaturityStrategy::RpcFinalized),
            Some(eth::Maturity::Tag(eth::BlockTag::Finalized))
        );
    }
}
