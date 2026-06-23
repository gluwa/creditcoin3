//! Outbox resolution (confluence §7.3 A2 / §2.2).
//!
//! Turns the attestor's `u64` `chain_key` into the concrete Creditcoin L1 Outbox to watch:
//!
//! 1. If an explicit `outbox_address` override is configured, use it (fastest PoC path).
//! 2. Otherwise resolve the factory address — config override first, then the on-chain `chain-info`
//!    precompile (`outbox_factory_address`, PR #873) — and call `IOutboxFactory.getOutbox(bytes32)`.
//!
//! The destination chain key is known locally (config override, else derived from the `u64`
//! `chain_key`); the attestor does not read it back from the Outbox.
//!
//! `getOutbox` returning `address(0)` is a fail-fast for the PoC (the production path will back off
//! and subscribe to `OutboxCreated`).

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use anyhow::{anyhow, bail, Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutboxFactory};
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
    /// The destination chain key bound into `messageHash` (PoC §5.2). Known locally from config
    /// (else derived from the `u64` `chain_key`) rather than read back from the Outbox.
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
    // The destination chain key is known locally: a `bytes32` override if configured, otherwise
    // derived from the `u64` `chain_key`. It is used both for the factory lookup and as the key
    // bound into every `messageHash` on this Outbox.
    let destination_chain_key = cfg
        .write_ability_chain_key
        .unwrap_or_else(|| chain_key_to_bytes32(chain_key));

    let address = if let Some(addr) = cfg.outbox_address {
        tracing::info!(%addr, "🧭 using configured Outbox address override");
        addr
    } else {
        let factory = resolve_factory(provider, chain_key, cfg).await?;
        let outbox = IOutboxFactory::new(factory, provider)
            .getOutbox(destination_chain_key)
            .call()
            .await
            .with_context(|| format!("IOutboxFactory.getOutbox at {factory} reverted"))?
            ._0;
        if outbox.is_zero() {
            bail!(
                "factory {factory} has no Outbox for chain_key {chain_key} \
                 (bytes32 {destination_chain_key}) yet"
            );
        }
        tracing::info!(%factory, %outbox, "🧭 resolved Outbox via factory");
        outbox
    };

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
