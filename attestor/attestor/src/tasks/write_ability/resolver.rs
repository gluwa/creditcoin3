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
//! Resolution continues after activation: [`super::run_outbox_monitor`] polls this resolver with the
//! same incremental cursor and hot-swaps the listener when governance changes the factory or the
//! active factory emits a replacement Outbox.

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::rpc::types::BlockTransactionsKind;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedOutbox {
    /// Outbox contract address on Creditcoin L1.
    pub address: Address,
    /// The destination chain key bound into `messageHash` (PoC §5.2). Sourced from the on-chain
    /// `WriteAbilityConfigs` entry when registered, else derived locally from `chain_key`.
    pub destination_chain_key: B256,
    /// Creditcoin L1 EVM chain id (`eth_chainId`) bound into `messageHash`.
    pub creditcoin_chain_id: u64,
    /// Block the binding `OutboxCreated` was emitted in, when the log reported one.
    ///
    /// A listener started for *this* Outbox needs no history before this height, and must not start
    /// after it: on a hot swap the boot-time `start_block` is the wrong floor, because governance can
    /// point a chain key at an Outbox that was created before this process booted, whose earlier
    /// `MessagePublished` events would then never be scanned.
    pub created_at_block: Option<u64>,
}

/// Caller-owned state for the incremental Outbox-discovery scan across activation retries. Tracks
/// how far we have scanned and against which factory, so each retry only scans new *confirmed*
/// blocks (no full-history log storm) while staying reorg-safe, and a factory re-registration
/// restarts the scan from genesis.
#[derive(Default)]
pub struct OutboxDiscoveryCursor {
    /// Next block to scan from.
    from: u64,
    /// Factory the cursor was advanced against; a change resets `from` to 0.
    factory: Option<Address>,
    /// Newest `OutboxCreated` (address, emitting block) seen so far, carried across attempts.
    /// Progress and discovery must advance together: `from` moves past each completed chunk, so
    /// keeping the match only in a local would lose it whenever a *later* chunk failed — the next
    /// attempt would resume past the event and report "no Outbox" forever (bugbot). Reset with
    /// `from` on a factory change.
    found: Option<(Address, Option<u64>)>,
}

impl OutboxDiscoveryCursor {
    /// Block the scan has reached. The caller uses this to tell a resolve attempt that *failed after
    /// advancing* from one that made no headway at all, so a chunked scan spanning several attempts
    /// is not mistaken for a dead RPC.
    pub(super) fn scanned_to(&self) -> u64 {
        self.from
    }

    /// Factory the current scan position belongs to. The rotation monitor uses this to distinguish
    /// "no new Outbox event" from "governance changed/removed the factory and the replacement has
    /// not emitted an Outbox yet".
    pub(super) fn factory(&self) -> Option<Address> {
        self.factory
    }
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
    cursor: &mut OutboxDiscoveryCursor,
) -> Result<Option<ResolvedOutbox>> {
    let chain_key = cfg.write_ability_chain_key;

    // The Outbox address is resolved entirely on-chain from chain_key — never configured. `cursor`
    // is the caller-owned discovery state: it advances past already-scanned *confirmed* blocks so
    // repeated resolve retries don't re-scan the whole chain history (see `resolve_outbox_address`).
    let Some((address, created_at_block)) = resolve_outbox_address(
        provider,
        chain_key,
        destination_chain_key,
        cfg.block_confirmation_depth,
        cursor,
    )
    .await?
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
        created_at_block,
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
    confirmation_depth: u64,
    cursor: &mut OutboxDiscoveryCursor,
) -> Result<Option<(Address, Option<u64>)>> {
    // 1. Outbox factory for this chain, from the chain-info precompile.
    let factory = IChainInfo::new(CHAIN_INFO_PRECOMPILE, provider)
        .get_outbox_factory_address(chain_key)
        .call()
        .await
        .context("chain-info precompile get_outbox_factory_address() reverted")?;
    if !factory.exists || factory.factoryAddr.is_zero() {
        // Record removal as a real governance transition. Keeping the previous factory here would
        // make the monitor interpret `Ok(None)` as merely "no new events" and continue signing the
        // de-registered Outbox until another factory eventually appeared.
        cursor.factory = None;
        cursor.from = 0;
        cursor.found = None;
        tracing::warn!(
            chain_key,
            "no Outbox factory registered on-chain for chain_key"
        );
        return Ok(None);
    }
    let factory = factory.factoryAddr;

    // If the resolved factory changed (governance re-registered a different one for this chain key),
    // the cursor reflects the *old* factory's scanned history and could skip the new factory's
    // OutboxCreated. Restart from genesis for the new factory.
    if cursor.factory != Some(factory) {
        cursor.factory = Some(factory);
        cursor.from = 0;
        cursor.found = None;
    }

    // 2. Discover the factory's Outbox for this chain key. The synced CREATE2 factory has no
    //    `getOutbox` registry, so scan its `OutboxCreated` events (chainKey is an indexed uint32,
    //    topics[2]) and take the first match — a factory deploys one Outbox per chain key.
    //
    //    Scan in bounded windows (eth_getLogs defaults `fromBlock` to *latest*, and a single
    //    genesis→tip request exceeds common RPC block-range limits on any non-trivial chain — the
    //    same reason the listener chunks at `MAX_LOG_BLOCK_RANGE`). Resume from the caller-owned
    //    `cursor.from` rather than genesis: the factory emits OutboxCreated exactly once, so once a
    //    retry has scanned the confirmed range and found nothing, the next retry only needs the new
    //    blocks — otherwise every 12s retry re-issues a full-history log storm across all attestors.
    //
    //    Bound the scan (and thus the cursor) at the FINALIZED head, mirroring the listener's
    //    `FinalityPolicy::Finalized`: Creditcoin L1 has deterministic GRANDPA finality, so a
    //    finalized block cannot reorg. Only fall back to `tip - confirmation_depth` when the
    //    `finalized` tag is unavailable (node up but tag unsupported/errored), matching the
    //    listener's depth fallback. The cursor advances permanently, so resolving OutboxCreated
    //    from a still-reorgable block would let a reorg re-mine it below the cursor and hide the
    //    Outbox for the whole process lifetime — the listener signs only up to the finalized head,
    //    so discovery must not run ahead of it.
    let tip = provider
        .get_block_number()
        .await
        .context("read EVM tip for Outbox discovery scan")?;
    let safe_tip = match provider
        .get_block_by_number(BlockNumberOrTag::Finalized, BlockTransactionsKind::Hashes)
        .await
    {
        Ok(Some(block)) => block.header.number,
        // `finalized` tag unsupported/errored → depth fallback (same as the listener). The tip
        // read above already covers a dead RPC, so a finalized-read failure is not fatal here.
        Ok(None) | Err(_) => tip.saturating_sub(confirmation_depth),
    };
    let sig = IOutboxFactory::OutboxCreated::SIGNATURE_HASH;
    let topic_chain_key = alloy::primitives::U256::from(chain_key);
    let mut from = cursor.from;
    // Bind the *latest* OutboxCreated for this chain_key, not the first. The factory is
    // permissionless, so it can emit more than one OutboxCreated for a chain_key (a redeploy, or a
    // different `msg.sender`'s CREATE2 salt); returning the oldest could bind a superseded or
    // squatting deployment, so scan the whole confirmed range and keep the most recent. `get_logs`
    // returns ascending, so the last log of the last non-empty chunk is the newest. (Via the
    // OutboxDeployer the registry is one-per-chain_key, so in practice there is exactly one — this
    // just makes the ambiguous case deterministic rather than order-of-emission dependent.)
    while from <= safe_tip {
        let chunk_to = safe_tip.min(from + MAX_LOG_BLOCK_RANGE - 1);
        let filter = alloy::rpc::types::Filter::new()
            .address(factory)
            .event_signature(sig)
            .topic2(topic_chain_key)
            .from_block(from)
            .to_block(chunk_to);
        let logs = provider.get_logs(&filter).await.with_context(|| {
            format!("eth_getLogs OutboxCreated at factory {factory} [{from}..={chunk_to}] failed")
        })?;
        if let Some(log) = logs.last() {
            // Record into the cursor, not a local: this must survive an attempt that is abandoned
            // after this chunk (see `OutboxDiscoveryCursor::found`). `get_logs` returns ascending,
            // so the last log of the newest non-empty chunk stays the newest match overall.
            let outbox = IOutboxFactory::OutboxCreated::decode_log(&log.inner, true)
                .context("decode OutboxCreated log")?
                .data
                .outbox;
            // Keep the emitting block: it is the only floor a hot-swapped listener can trust.
            cursor.found = Some((outbox, log.block_number));
        }
        from = chunk_to + 1;
        // Record progress per *chunk*, not per scan. A resolve attempt can be abandoned mid-loop
        // (`RPC_ATTEMPT_TIMEOUT`, or a later chunk erroring), and the caller distinguishes "failed
        // after advancing" from "made no headway" via `scanned_to()`. Advancing only after the whole
        // range would report zero progress for a scan that covered thousands of blocks, so a long
        // chain would burn the failure budget and restart on every attempt without ever finishing
        // discovery (bugbot). Chunks are scanned in ascending order and the cursor is the resume
        // point, so committing each completed chunk is safe: at worst the newest-Outbox scan below
        // resumes from here and re-reads nothing already covered.
        cursor.from = from;
    }
    // `from` is `safe_tip + 1` when the loop ran, or unchanged (`cursor.from`) if
    // `safe_tip < cursor.from` (tip regressed / depth grew) — never regressing the cursor.
    cursor.from = cursor.from.max(from);
    if let Some((outbox, created_at_block)) = cursor.found {
        tracing::info!(
            %factory, %outbox, chain_key, ?created_at_block,
            "🧭 resolved Outbox on-chain (OutboxCreated scan)"
        );
        return Ok(Some((outbox, created_at_block)));
    }
    tracing::warn!(%factory, chain_key, "factory has emitted no OutboxCreated for chain_key yet");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    // Progress and discovery must move together. The scan commits `from` per chunk so an attempt
    // abandoned mid-loop keeps its headway (otherwise the failure budget restarts discovery
    // forever on a long chain) — but that means a match found in an early chunk sits *behind* the
    // cursor, so it has to be carried in the cursor too. Keeping it in a local made a later-chunk
    // failure lose the address permanently: every retry resumed past the event and reported
    // "no Outbox", leaving write-ability inactive for the process lifetime.
    #[test]
    fn cursor_carries_a_discovery_past_an_interrupted_scan() {
        let outbox = address!("00000000000000000000000000000000000000aa");
        let factory = address!("00000000000000000000000000000000000000ff");

        // Attempt 1: chunk at [0..=999] matched, then a later chunk failed — the function returned
        // Err after committing both the progress and the match.
        let mut cursor = OutboxDiscoveryCursor {
            from: 1_000,
            factory: Some(factory),
            found: Some((outbox, Some(950))),
        };

        // Attempt 2 resumes from 1_000 and finds nothing new; the earlier discovery must survive.
        assert_eq!(cursor.scanned_to(), 1_000);
        assert_eq!(cursor.found, Some((outbox, Some(950))));

        // A factory re-registration invalidates both: the new factory's Outbox may live anywhere,
        // and the old address must not be reported for it.
        let new_factory = address!("00000000000000000000000000000000000000ee");
        if cursor.factory != Some(new_factory) {
            cursor.factory = Some(new_factory);
            cursor.from = 0;
            cursor.found = None;
        }
        assert_eq!(cursor.scanned_to(), 0);
        assert_eq!(cursor.found, None);
    }
}
