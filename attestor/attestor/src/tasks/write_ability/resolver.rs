//! Outbox resolution (confluence §7.3 A2 / §2.2).
//!
//! Turns the attestor's `u64` `chain_key` into the concrete Creditcoin L1 Outbox to watch, by
//! reading the discovery-registry address off the `chain-info` precompile and asking that registry
//! for `defaultOutbox(chainKey)` — see [`resolve_outbox_from_registry`] for why this, and not a scan
//! of the permissionless factory's `OutboxCreated` logs, is the only resolution path.
//!
//! The destination chain key (`bytes32`) is supplied by the caller — sourced from the on-chain
//! `WriteAbilityConfigs` entry when registered, else derived locally from the attestor's
//! `chain_key` — and bound into `messageHash`, never read back from the Outbox.
//!
//! Activation is dynamic: the write-ability task ([`super::run`]) retries [`resolve`] on a timer, so
//! an attestor started before a discovery address is registered for its chain key activates
//! automatically once one is — no restart needed. It runs normal block attestation in the meantime.
//!
//! Resolution continues after activation: [`super::run_outbox_monitor`] polls [`resolve`] on the
//! same cadence and hot-swaps the listener when the registry's `defaultOutbox` answer changes
//! (governance called `setDefaultOutbox`/`removeOutbox` on `OutboxDiscovery`, or the on-chain
//! discovery-registry address itself was re-pointed).
//!
//! A registry read is complete and authoritative on every call — there is no scan to resume and
//! nothing to persist across restarts, unlike the `OutboxCreated` log scan this module used to run.

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use anyhow::{Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutboxDiscovery};

/// `chain-info` precompile address (`0x…0fD3`, 4051) — see `precompiles/metadata/sol/chain_info.sol`.
/// Exposes `pallet_supported_chains::OutboxDiscoveries` (`chain_key → discovery-registry address`)
/// to the EVM.
pub const CHAIN_INFO_PRECOMPILE: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f, 0xd3,
]);

/// The Outbox an attestor watches, plus the immutable inputs every `messageHash` on it binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedOutbox {
    /// Outbox contract address on Creditcoin L1.
    pub address: Address,
    /// The destination chain key bound into `messageHash` (PoC §5.2). Sourced from the on-chain
    /// `WriteAbilityConfigs` entry when registered, else derived locally from `chain_key`.
    pub destination_chain_key: B256,
    /// Creditcoin L1 EVM chain id (`eth_chainId`) bound into `messageHash`.
    pub creditcoin_chain_id: u64,
}

/// Resolve the Outbox for the configured write-ability chain key using `provider` (a Creditcoin L1
/// EVM connection). `destination_chain_key` is the effective `bytes32` key (see
/// [`super::MessageVoteState::destination_chain_key`]) used both to ask the registry for its Outbox
/// and as the hash-binding key.
///
/// Returns `Ok(None)` when no discovery-registry address is registered on-chain for this chain key
/// yet, or the registry has no default Outbox for it — the caller treats that as "write-ability not
/// available" and disables it for the run rather than failing. `Err` is reserved for genuine
/// RPC/contract failures.
pub async fn resolve<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
    destination_chain_key: B256,
) -> Result<Option<ResolvedOutbox>> {
    let Some(address) = resolve_outbox_from_registry(provider, chain_key).await? else {
        return Ok(None);
    };

    let creditcoin_chain_id = provider
        .get_chain_id()
        .await
        .context("failed to read Creditcoin L1 EVM chain id")?;

    Ok(Some(ResolvedOutbox {
        address,
        destination_chain_key,
        creditcoin_chain_id,
    }))
}

/// Resolve the Outbox by reading the discovery-registry address off the chain-info precompile and
/// calling `defaultOutbox` on it. This is the only resolution path: [`resolve`] no longer falls back
/// to scanning the permissionless factory's `OutboxCreated` logs.
///
/// This exists for a security reason, not as a shortcut: `OutboxFactory.deployOutbox` is
/// intentionally permissionless, so any account can deploy an Outbox for `chain_key` and emit an
/// `OutboxCreated` indistinguishable from a legitimate one — binding the newest such event would let
/// an attacker's deployment become permanently newest. The registry (`OutboxDiscovery` in
/// asc-contracts, merged as asc-contracts#38) is written only through an access-controlled deploy
/// path, so its `defaultOutbox` answer is safe to trust directly.
///
/// `defaultOutbox`, not `outboxOf`: confirmed as the source of truth on Slack (Kevin Nguyen,
/// 28 Aug 2026) — "the default deployed outbox for each chain via: `defaultOutbox(chainKey)` (not
/// from deployer) because there will be multiple version[s] of outbox".
///
/// Returns `Ok(None)` — not an error — when no discovery address is registered for `chain_key` yet,
/// or the registry has no default Outbox for it. The caller treats that as "not yet activated" and
/// keeps retrying rather than failing the run.
async fn resolve_outbox_from_registry<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
) -> Result<Option<Address>> {
    let discovery = IChainInfo::new(CHAIN_INFO_PRECOMPILE, provider)
        .get_outbox_discovery_address(chain_key)
        .call()
        .await
        .context("chain-info precompile get_outbox_discovery_address() reverted")?;
    if !discovery.exists || discovery.discoveryAddr.is_zero() {
        return Ok(None);
    }
    let discovery = discovery.discoveryAddr;

    // The registry contract keys `defaultOutbox` by `uint32` while `chain_key` is `u64`. Reject
    // anything unrepresentable rather than truncating: a silently wrapped key would read the
    // registry for a *different* chain and bind whatever Outbox that answered with.
    let chain_key_u32 = u32::try_from(chain_key).with_context(|| {
        format!(
            "chain_key {chain_key} exceeds the uint32 the Outbox registry is keyed by, so it \
             cannot be represented on-chain"
        )
    })?;

    let outbox = IOutboxDiscovery::new(discovery, provider)
        .defaultOutbox(chain_key_u32)
        .call()
        .await
        .with_context(|| format!("defaultOutbox reverted on registry {discovery}"))?;

    if outbox._0.is_zero() {
        return Ok(None);
    }

    Ok(Some(outbox._0))
}
