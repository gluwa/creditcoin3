//! Outbox resolution (confluence §7.3 A2 / §2.2).
//!
//! Turns the attestor's `u64` `chain_key` into the concrete Creditcoin L1 Outbox to watch. The
//! factory and Outbox addresses are resolved entirely on-chain from `chain_key`; they are
//! deliberately not configurable, because an address supplied separately from the chain key may
//! not correspond to it.
//!
//! The destination chain key (`bytes32`) is supplied by the caller — sourced from the on-chain
//! `WriteAbilityConfigs` entry when registered, else derived locally from the attestor's
//! `chain_key` — and bound into `messageHash`, never read back from the Outbox.
//!
//! Activation is dynamic: the write-ability task ([`super::run`]) retries `resolve` on a timer, so
//! an attestor started before the factory/Outbox is registered activates automatically once they
//! exist — no restart needed. It runs normal block attestation in the meantime.
//!
//! TODO(write-ability): pick up a later *re-registration* (factory address change / new Outbox)
//! mid-run. The polling activation above only fires until the first successful resolve; reacting to
//! changes after that would mean subscribing (via the cc3 client) to the `OutboxFactoryRegistered`
//! event (`pallets/supported-chains/src/lib.rs`) and the `OutboxCreated` event
//! (`common/write-ability/src/abi.rs`) and re-resolving.

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutboxFactory};

use super::config::Config;

/// `chain-info` precompile address (`0x…0fD3`, 4051) — see `precompiles/metadata/sol/chain_info.sol`.
/// Exposes `pallet_supported_chains::OutboxFactories` (`chain_key → factory address`) to the EVM.
pub const CHAIN_INFO_PRECOMPILE: Address = Address::new([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x0f, 0xd3,
]);

/// Max block span per `eth_getLogs` request when scanning the factory for `OutboxCreated` — an
/// unbounded genesis→tip span exceeds common RPC limits. Mirrors the listener's chunk size.
const MAX_LOG_BLOCK_RANGE: u64 = 2000;

/// The Outbox an attestor watches, plus the immutable inputs every `messageHash` on it binds.
#[derive(Clone, Copy, Debug)]
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
/// [`super::MessageVoteState::destination_chain_key`]) used both to ask the factory for its Outbox
/// and as the hash-binding key.
///
/// Returns `Ok(None)` when no Outbox factory / Outbox is registered on-chain for this chain key —
/// the caller treats that as "write-ability not available" and disables it for the run rather than
/// failing. `Err` is reserved for genuine RPC/contract failures.
pub async fn resolve<P: Provider>(
    provider: &P,
    cfg: &Config,
    destination_chain_key: B256,
) -> Result<Option<ResolvedOutbox>> {
    let chain_key = cfg.write_ability_chain_key;

    // The Outbox address is resolved entirely on-chain from chain_key — never configured.
    let Some(address) = resolve_outbox_address(provider, chain_key, destination_chain_key).await?
    else {
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
    _destination_chain_key: B256,
) -> Result<Option<Address>> {
    // 1. Outbox factory for this chain, from the chain-info precompile.
    let factory = IChainInfo::new(CHAIN_INFO_PRECOMPILE, provider)
        .get_outbox_factory_address(chain_key)
        .call()
        .await
        .context("chain-info precompile get_outbox_factory_address() reverted")?;
    if !factory.exists || factory.factoryAddr.is_zero() {
        tracing::warn!(
            chain_key,
            "no Outbox factory registered on-chain for chain_key"
        );
        return Ok(None);
    }
    let factory = factory.factoryAddr;

    // 2. Discover the factory's Outbox for this chain key. The synced CREATE2 factory has no
    //    `getOutbox` registry, so scan its `OutboxCreated` events (chainKey is an indexed uint32,
    //    topics[2]) and take the first match — a factory deploys one Outbox per chain key.
    //
    //    Scan from genesis (eth_getLogs defaults `fromBlock` to *latest*, which would only ever see
    //    an OutboxCreated mined in the current block), but in bounded windows: a single
    //    genesis→tip request exceeds common RPC block-range limits on any non-trivial chain — the
    //    same reason the listener chunks at `MAX_LOG_BLOCK_RANGE`.
    let tip = provider
        .get_block_number()
        .await
        .context("read EVM tip for Outbox discovery scan")?;
    let sig = IOutboxFactory::OutboxCreated::SIGNATURE_HASH;
    let topic_chain_key = alloy::primitives::U256::from(chain_key);
    let mut from = 0u64;
    while from <= tip {
        let chunk_to = tip.min(from + MAX_LOG_BLOCK_RANGE - 1);
        let filter = alloy::rpc::types::Filter::new()
            .address(factory)
            .event_signature(sig)
            .topic2(topic_chain_key)
            .from_block(from)
            .to_block(chunk_to);
        let logs = provider.get_logs(&filter).await.with_context(|| {
            format!("eth_getLogs OutboxCreated at factory {factory} [{from}..={chunk_to}] failed")
        })?;
        if let Some(log) = logs.first() {
            let outbox = IOutboxFactory::OutboxCreated::decode_log(&log.inner, true)
                .context("decode OutboxCreated log")?
                .data
                .outbox;
            tracing::info!(%factory, %outbox, chain_key, "🧭 resolved Outbox on-chain (OutboxCreated scan)");
            return Ok(Some(outbox));
        }
        from = chunk_to + 1;
    }
    tracing::warn!(%factory, chain_key, "factory has emitted no OutboxCreated for chain_key yet");
    Ok(None)
}
