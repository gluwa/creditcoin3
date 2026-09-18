//! Historical Discovery authorization for write-ability messages.
//!
//! A default Outbox is a convenience for publishers, not an attestation allowlist. Every Outbox
//! registered for the chain may publish until its removal becomes effective. Resolve that authority
//! at the message's finalized source block: querying today's default or active set loses legitimate
//! history after a default change, removal, re-registration, or Discovery registry replacement.

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use anyhow::{Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutboxDiscovery};

/// `chain-info` precompile address (`0x…0fD3`, 4051).
pub const CHAIN_INFO_PRECOMPILE: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f, 0xd3,
]);

/// Routing inputs shared by every authorized Outbox for this destination.
///
/// Deliberately contains no current default/registry address: neither is a reason to discard an
/// already finalized message or reset the scan cursor. Historical authority is checked separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedRoute {
    pub chain_key: ChainKey,
    pub destination_chain_key: B256,
    pub creditcoin_chain_id: u64,
}

/// Resolve the immutable signing domain. The listener can start before any Outbox is registered;
/// every message still requires a successful historical Discovery membership check.
/// `Option` allows the caller's governance gate to suspend this route.
pub async fn resolve<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
    destination_chain_key: B256,
) -> Result<Option<ResolvedRoute>> {
    registry_chain_key(chain_key)?;
    let creditcoin_chain_id = provider
        .get_chain_id()
        .await
        .context("failed to read Creditcoin L1 EVM chain id")?;
    Ok(Some(ResolvedRoute {
        chain_key,
        destination_chain_key,
        creditcoin_chain_id,
    }))
}

fn registry_chain_key(chain_key: ChainKey) -> Result<u32> {
    u32::try_from(chain_key)
        .with_context(|| format!("chain_key {chain_key} exceeds the uint32 Outbox registry key"))
}

/// Find the governance-registered Discovery at a particular source block. An RPC error is never
/// equivalent to an empty registry: callers must retry without advancing their cursor.
pub async fn discovery_at<P: Provider>(
    provider: &P,
    route: &ResolvedRoute,
    block: u64,
) -> Result<Option<Address>> {
    let discovery = IChainInfo::new(CHAIN_INFO_PRECOMPILE, provider)
        .get_outbox_discovery_address(route.chain_key)
        .block(BlockNumberOrTag::Number(block).into())
        .call()
        .await
        .context("historical chain-info Discovery lookup failed")?;
    Ok((discovery.exists && !discovery.discoveryAddr.is_zero()).then_some(discovery.discoveryAddr))
}

/// Membership at the message's source block handles scheduled removals (exclusive effective
/// block), cancellations, and later re-registration using the contract's own lifecycle semantics.
/// Historical state is required; an RPC unable to serve it must fail closed.
pub async fn authorized_at<P: Provider>(
    provider: &P,
    route: &ResolvedRoute,
    discovery: Address,
    outbox: Address,
    block: u64,
) -> Result<bool> {
    Ok(IOutboxDiscovery::new(discovery, provider)
        .isActiveOutbox(registry_chain_key(route.chain_key)?, outbox)
        .block(BlockNumberOrTag::Number(block).into())
        .call()
        .await
        .context("historical Discovery membership lookup failed")?
        ._0)
}
