//! Outbox resolution (confluence §7.3 A2 / §2.2).
//!
//! Turns the attestor's `u64` `chain_key` into the concrete Creditcoin L1 Outbox to watch. The
//! factory and Outbox addresses are resolved entirely on-chain from `chain_key`; they are
//! deliberately not configurable, because an address supplied separately from the chain key may
//! not correspond to it.
//!
//! The destination chain key is the attestor's configured write-ability chain key; its `bytes32`
//! form is computed locally and bound into `messageHash`, never read back from the Outbox.
//!
//! TODO(write-ability): support dynamic (re)discovery of the Outbox factory + Outbox while the
//! attestor is running. Today resolution happens once at startup, so an attestor must be restarted
//! after the factory is registered / the Outbox is created. Instead, an attestor with write-ability
//! activated should run normally with signing disabled when no factory/Outbox is configured yet,
//! then activate write-ability signing automatically once they are created — and likewise pick up
//! later additions/changes. It would learn of these by subscribing (via the cc3 client) to the
//! `OutboxFactoryRegistered` event (`pallets/supported-chains/src/lib.rs`) and the `OutboxCreated`
//! event (`common/write-ability/src/abi.rs`).

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use anyhow::{Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutboxFactory};
use write_ability::protocol::chain_key_to_bytes32;

use super::config::Config;

/// `chain-info` precompile address (`0x…0fD3`, 4051) — see `precompiles/metadata/sol/chain_info.sol`.
/// Exposes `pallet_supported_chains::OutboxFactories` (`chain_key → factory address`) to the EVM.
pub const CHAIN_INFO_PRECOMPILE: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f, 0xd3,
]);

/// The Outbox an attestor watches, plus the immutable inputs every `messageHash` on it binds.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedOutbox {
    /// Outbox contract address on Creditcoin L1.
    pub address: Address,
    /// The destination chain key bound into `messageHash` (PoC §5.2). The `bytes32` form of the
    /// attestor's configured write-ability chain key, computed locally rather than read from chain.
    pub destination_chain_key: B256,
    /// Creditcoin L1 EVM chain id (`eth_chainId`) bound into `messageHash`.
    pub creditcoin_chain_id: u64,
}

/// Resolve the Outbox for the configured write-ability chain key using `provider` (a Creditcoin L1
/// EVM connection).
///
/// Returns `Ok(None)` when no Outbox factory / Outbox is registered on-chain for this chain key —
/// the caller treats that as "write-ability not available" and disables it for the run rather than
/// failing. `Err` is reserved for genuine RPC/contract failures.
pub async fn resolve<P: Provider>(provider: &P, cfg: &Config) -> Result<Option<ResolvedOutbox>> {
    let chain_key = cfg.write_ability_chain_key;

    // The Outbox address is resolved entirely on-chain from chain_key — never configured.
    let Some(address) = resolve_outbox_address(provider, chain_key).await? else {
        return Ok(None);
    };

    let creditcoin_chain_id = provider
        .get_chain_id()
        .await
        .context("failed to read Creditcoin L1 EVM chain id")?;

    Ok(Some(ResolvedOutbox {
        address,
        destination_chain_key: chain_key_to_bytes32(chain_key),
        creditcoin_chain_id,
    }))
}

/// Resolve the Outbox contract address for `chain_key` entirely on-chain.
///
/// 1. Fetch the Outbox factory for this chain from the `chain-info` precompile, which exposes
///    `pallet_supported_chains::OutboxFactories` (a `chain_key → factory address` map) to the EVM.
/// 2. Ask that factory for the Outbox bound to this chain key via `IOutboxFactory.getOutbox`.
///
/// Returns `Ok(None)` when no factory is registered for `chain_key` or the factory has no Outbox for
/// it yet (the on-chain values are zero). Neither address is configurable — supplying one separately
/// from the chain key is error prone, since it might not correspond to that chain key.
async fn resolve_outbox_address<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
) -> Result<Option<Address>> {
    // 1. Outbox factory for this chain, from the chain-info precompile.
    let factory = IChainInfo::new(CHAIN_INFO_PRECOMPILE, provider)
        .outbox_factory_address(chain_key)
        .call()
        .await
        .context("chain-info precompile outbox_factory_address() reverted")?;
    if !factory.exists || factory.factory_addr.is_zero() {
        tracing::warn!(
            chain_key,
            "no Outbox factory registered on-chain for chain_key"
        );
        return Ok(None);
    }
    let factory = factory.factory_addr;

    // 2. The factory's Outbox for this chain key.
    let outbox = IOutboxFactory::new(factory, provider)
        .getOutbox(chain_key_to_bytes32(chain_key))
        .call()
        .await
        .with_context(|| format!("IOutboxFactory.getOutbox at {factory} reverted"))?
        ._0;
    if outbox.is_zero() {
        tracing::warn!(%factory, chain_key, "factory has no Outbox for chain_key yet");
        return Ok(None);
    }
    tracing::info!(%factory, %outbox, chain_key, "🧭 resolved Outbox on-chain");
    Ok(Some(outbox))
}
