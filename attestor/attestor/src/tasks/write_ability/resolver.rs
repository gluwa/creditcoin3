//! Outbox resolution (confluence §7.3 A2 / §2.2).
//!
//! Turns the attestor's `u64` `chain_key` into the concrete Creditcoin L1 Outbox to watch:
//!
//! 1. If an explicit `outbox_address` override is configured, use it (fastest PoC path).
//! 2. Otherwise resolve the factory address — config override first, then the on-chain `chain-info`
//!    precompile (`outbox_factory_address`, PR #873) — and call `IOutboxFactory.getOutbox(bytes32)`.
//! 3. Read `Outbox.chainKey()` and, when a `bytes32` key is configured, assert it matches.
//!
//! `getOutbox` returning `address(0)` is a fail-fast for the PoC (the production path will back off
//! and subscribe to `OutboxCreated`).

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use anyhow::{anyhow, bail, Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutbox, IOutboxFactory};
use write_ability::protocol::chain_key_to_bytes32;

use super::config::Config;

/// `chain-info` precompile address (`0x…0fD3`, 4051) — see `precompiles/metadata/sol/chain_info.sol`.
pub const CHAIN_INFO_PRECOMPILE: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f, 0xd3,
]);

/// The Outbox an attestor watches, plus the immutable inputs every `messageHash` on it binds.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedOutbox {
    /// Outbox contract address on Creditcoin L1.
    pub address: Address,
    /// `Outbox.chainKey()` — the destination chain key bound into `messageHash` (PoC §5.2).
    pub destination_chain_key: B256,
    /// Creditcoin L1 EVM chain id (`eth_chainId`) bound into `messageHash`.
    pub creditcoin_chain_id: u64,
}

/// Resolve the Outbox for `chain_key` using `provider` (a Creditcoin L1 EVM connection).
pub async fn resolve<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
    cfg: &Config,
) -> Result<ResolvedOutbox> {
    let address = if let Some(addr) = cfg.outbox_address {
        tracing::info!(%addr, "🧭 using configured Outbox address override");
        addr
    } else {
        let factory = resolve_factory(provider, chain_key, cfg).await?;
        let key_b32 = cfg
            .write_ability_chain_key
            .unwrap_or_else(|| chain_key_to_bytes32(chain_key));
        let outbox = IOutboxFactory::new(factory, provider)
            .getOutbox(key_b32)
            .call()
            .await
            .with_context(|| format!("IOutboxFactory.getOutbox at {factory} reverted"))?
            ._0;
        if outbox.is_zero() {
            bail!(
                "factory {factory} has no Outbox for chain_key {chain_key} (bytes32 {key_b32}) yet"
            );
        }
        tracing::info!(%factory, %outbox, "🧭 resolved Outbox via factory");
        outbox
    };

    let destination_chain_key = IOutbox::new(address, provider)
        .chainKey()
        .call()
        .await
        .with_context(|| format!("Outbox.chainKey() at {address} reverted"))?
        ._0;

    if let Some(expected) = cfg.write_ability_chain_key {
        if expected != destination_chain_key {
            bail!(
                "Outbox.chainKey() {destination_chain_key} != configured write_ability_chain_key {expected}"
            );
        }
    }

    let creditcoin_chain_id = provider
        .get_chain_id()
        .await
        .context("failed to read Creditcoin L1 EVM chain id")?;

    Ok(ResolvedOutbox {
        address,
        destination_chain_key,
        creditcoin_chain_id,
    })
}

/// Resolve the factory address: config override first, then the on-chain `chain-info` precompile.
async fn resolve_factory<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
    cfg: &Config,
) -> Result<Address> {
    if let Some(addr) = cfg.outbox_factory_address {
        return Ok(addr);
    }
    let result = IChainInfo::new(CHAIN_INFO_PRECOMPILE, provider)
        .outbox_factory_address(chain_key)
        .call()
        .await
        .context("chain-info precompile outbox_factory_address() reverted")?;
    if !result.exists || result.factory_addr.is_zero() {
        return Err(anyhow!(
            "no outbox factory registered on-chain for chain_key {chain_key}; set one via \
             SupportedChains::set_outbox_factory_addr or configure outbox_factory_address"
        ));
    }
    Ok(result.factory_addr)
}
