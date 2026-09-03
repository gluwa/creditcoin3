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
//!
//! The `OutboxCreated` scan this module runs is itself durable: when a
//! [`FactoryScanCursorStore`](super::cursor::FactoryScanCursorStore) is supplied, progress is
//! persisted after every call and — via [`OutboxDiscoveryCursor::from_persisted`] — reloaded once
//! at process start, so a restart resumes the scan instead of rescanning the factory's full log
//! history from genesis. A factory change (rotation) used to force that same genesis rescan even
//! within one run; with [`Config::resume_rotation_from_checkpoint`] (on by default) it instead
//! resumes from the last checkpoint, same as a restart does.
//!
//! Resuming can overshoot: the checkpoint is a valid floor only when the rotated-to factory was
//! itself deployed at rotation time. Point a chain key at a factory whose `OutboxCreated` already
//! sits *below* the checkpoint and the resumed scan runs to the tip, matches nothing, and reports
//! "no Outbox" — permanently, since the cursor is then persisted at the tip against that very
//! factory and reads back as valid on the next boot. [`genesis_fallback_target`] closes that: a
//! scan that completes with no match, having started above [`Config::factory_scan_genesis_block`],
//! rewinds to that floor and tries once more before the chain key is declared Outbox-less.
//!
//! That fallback would otherwise fire on the *common*, healthy rotation too: a factory that was
//! itself just deployed looks identical, for one tick, to a pre-existing one — both complete a
//! scan above the floor and find nothing, because the fresh factory's own event has not finalized
//! yet. [`OutboxDiscoveryCursor::grace_until_tip`] holds the fallback off until the finalized head
//! has caught up to the tip observed when the floor was (re)armed, so a same-tick emission gets a
//! chance to finalize before its absence is trusted as evidence.

use alloy::primitives::{Address, B256};
use alloy::providers::Provider;
use alloy::rpc::types::eth::BlockNumberOrTag;
use alloy::rpc::types::BlockTransactionsKind;
use alloy::sol_types::SolEvent;
use anyhow::{Context, Result};

use attestor_primitives::ChainKey;
use write_ability::abi::{IChainInfo, IOutboxDiscovery, IOutboxFactory};

use super::config::Config;
use super::cursor::{FactoryScanCursorStore, PersistedFactoryScan};

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

/// Where a completed-but-empty `OutboxCreated` scan should restart, or `None` to accept "this
/// factory has no Outbox" as final. Pure so the decision is unit-testable without a live provider,
/// mirroring how the rest of this module's cursor arithmetic is tested.
///
/// The fallback is deliberately **not** configurable. The state it recovers from — a scan resumed
/// above the event it was looking for — is otherwise unrecoverable without an operator deleting the
/// cursor file on every pod by hand, and the recovery costs one extra scan of a range that was
/// going to be scanned anyway had the checkpoint not been trusted.
///
/// Two conditions, both necessary:
/// - `found_something` is false — a match makes the scan conclusive whatever floor it began at;
/// - `scan_floor > genesis_block` — the scan started above the floor, so there is unlooked-at range
///   below it. A scan that already began at the floor has covered everything and its "no Outbox"
///   verdict is the truth.
///
/// `grace_satisfied` guards against a race, not against re-firing: a rotation onto a factory that
/// was itself *just* deployed looks identical, for one tick, to a rotation onto a pre-existing one
/// whose event will never be found above the checkpoint — both complete a scan above the floor and
/// find nothing, because the fresh factory's own event has not finalized yet. Trusting that first
/// empty result would burn the one-shot recovery on the common, healthy rotation path this fallback
/// is not meant to touch. The caller only passes `true` once the finalized tip has caught up to the
/// height observed when the current floor was set, so a same-tick emission has had a chance to
/// finalize — see [`OutboxDiscoveryCursor::grace_until_tip`].
///
/// Nothing here bounds this to *one* rewind per factory: once it fires, the caller lowers
/// `scan_floor` to `genesis_block`, and every later call with the same (constant, per-process)
/// `genesis_block` finds `scan_floor > genesis_block` already false — the floor itself carries the
/// termination invariant.
fn genesis_fallback_target(
    found_something: bool,
    scan_floor: u64,
    genesis_block: u64,
    grace_satisfied: bool,
) -> Option<u64> {
    (!found_something && scan_floor > genesis_block && grace_satisfied).then_some(genesis_block)
}

/// The scan-shaping knobs [`resolve_outbox_address`] reads out of [`Config`], grouped so the
/// function keeps a legible signature as they accumulate (and stays under clippy's argument-count
/// limit). Copied per call — all three are plain scalars read from a config that only changes on
/// restart.
#[derive(Clone, Copy, Debug)]
struct DiscoveryPolicy {
    /// Depth below the tip to treat as safe when the `finalized` tag is unavailable.
    confirmation_depth: u64,
    /// See [`Config::resume_rotation_from_checkpoint`].
    resume_rotation_from_checkpoint: bool,
    /// See [`Config::factory_scan_genesis_block`] — the floor every from-scratch scan starts at.
    genesis_block: u64,
}

/// Caller-owned state for the incremental Outbox-discovery scan across activation retries. Tracks
/// how far we have scanned and against which factory, so each retry only scans new *confirmed*
/// blocks (no full-history log storm) while staying reorg-safe, and a factory re-registration
/// restarts the scan from the configured floor.
pub struct OutboxDiscoveryCursor {
    /// Next block to scan from.
    from: u64,
    /// Factory the cursor was advanced against; a change resets `from` to the scan floor.
    factory: Option<Address>,
    /// Newest `OutboxCreated` (address, emitting block) seen so far, carried across attempts.
    /// Progress and discovery must advance together: `from` moves past each completed chunk, so
    /// keeping the match only in a local would lose it whenever a *later* chunk failed — the next
    /// attempt would resume past the event and report "no Outbox" forever (bugbot). Reset with
    /// `from` on a factory change.
    found: Option<(Address, Option<u64>)>,
    /// Whether the most recent `resolve` call completed at least one log-scan chunk, reset to
    /// `false` at the start of every call. Callers use this — not a before/after diff of
    /// `scanned_to()` — to tell a resolve attempt that *failed after advancing* from one that made
    /// no headway at all: with `resume_rotation_from_checkpoint: false`, a rotation resets `from`
    /// back to the scan floor inside the same call — as does the genesis fallback — so a
    /// before/after diff would see a *lower* value after a call that did make progress and
    /// misreport it as a stall (bugbot).
    advanced_last_call: bool,
    /// Block the *current* factory's scan started from. Everything below it has never been looked
    /// at for this factory, which is exactly what the genesis fallback needs to know: a completed
    /// scan that found nothing is only conclusive when this equals the configured floor.
    ///
    /// Moves with `from` whenever a scan (re)starts — initial seed, rotation, de-registration,
    /// fallback — and is persisted so the distinction survives a restart.
    scan_floor: u64,
    /// Finalized tip that must be reached before a completed empty scan above `scan_floor` is
    /// trusted enough to trigger the genesis fallback, or `None` when no such wait is pending.
    ///
    /// Set to the *unfinalized* tip observed at the moment a rotation (re)armed the floor: any
    /// transaction at or before that height — including the new factory's own `OutboxCreated`, had
    /// it been emitted around rotation time — is already included in it, so once the finalized head
    /// reaches this value the absence of a match is evidence, not a race against finality. Without
    /// this, a rotation onto a factory that was itself *just* deployed looks identical, for one
    /// tick, to a rotation onto a pre-existing one whose event will never be found above the
    /// checkpoint — both complete a scan and find nothing, and only one of them should burn the
    /// one-shot recovery.
    grace_until_tip: Option<u64>,
    /// Set by [`Self::from_persisted`]: a reloaded cursor's floor may be a rotation that happened
    /// moments before the restart, but no tip is available yet to arm `grace_until_tip` against
    /// (that needs a live read). The next call converts this into `grace_until_tip` using the tip
    /// it reads, instead of trusting the reload's first scan immediately — the same race
    /// `grace_until_tip` guards against, just deferred one call because construction has no
    /// provider to ask.
    arm_grace_on_next_call: bool,
}

impl OutboxDiscoveryCursor {
    /// Seed a cursor from previously persisted scan progress (see
    /// [`FactoryScanCursorStore::load`](super::cursor::FactoryScanCursorStore::load)), so a restart
    /// resumes `OutboxCreated` discovery instead of starting over from genesis.
    ///
    /// The seed is provisional, not validated here: `resolve_outbox_address`'s on-chain factory
    /// read discards it via the ordinary `cursor.factory != factory` comparison below if the
    /// persisted factory turns out to differ from what is currently registered (a rotation that
    /// happened while the process was down) — the same path a mid-run rotation already takes.
    pub(super) fn from_persisted(persisted: PersistedFactoryScan) -> Self {
        Self {
            from: persisted.scanned_to,
            factory: Some(persisted.factory),
            found: persisted.found,
            advanced_last_call: false,
            scan_floor: persisted.scan_floor,
            grace_until_tip: None,
            // A restart cannot rule out that the persisted floor is a rotation from moments before
            // the process went down — arm the same grace the live rotation path would have, once a
            // tip is available to arm it against.
            arm_grace_on_next_call: true,
        }
    }

    /// A fresh cursor for a chain key with nothing persisted, starting at `genesis_block` (see
    /// [`Config::factory_scan_genesis_block`]).
    pub(super) fn starting_at(genesis_block: u64) -> Self {
        Self {
            from: genesis_block,
            factory: None,
            found: None,
            advanced_last_call: false,
            scan_floor: genesis_block,
            grace_until_tip: None,
            arm_grace_on_next_call: false,
        }
    }

    /// Reset the cursor for a factory rotation: `resume_from` is the new scan position (the old
    /// checkpoint when [`Config::resume_rotation_from_checkpoint`] is on, else the configured
    /// floor — see the call site), and `tip` is the *unfinalized* tip read in the same call, which
    /// arms [`Self::grace_until_tip`] against the exact race documented there.
    ///
    /// The one place this matters and testing can't otherwise reach: `resolve_outbox_address` needs
    /// a live `Provider`, so this method is what lets a unit test drive the real rotation reset
    /// instead of hand-copying its field assignments — the drift that copy is prone to is exactly
    /// what let this line go untested before (see the tests below).
    pub(super) fn rotate_to(&mut self, factory: Address, resume_from: u64, tip: u64) {
        self.factory = Some(factory);
        self.from = resume_from;
        self.scan_floor = resume_from;
        self.found = None;
        self.grace_until_tip = Some(tip);
        self.arm_grace_on_next_call = false;
    }

    /// Block the scan has reached.
    pub(super) fn scanned_to(&self) -> u64 {
        self.from
    }

    /// Factory the current scan position belongs to. The rotation monitor uses this to distinguish
    /// "no new Outbox event" from "governance changed/removed the factory and the replacement has
    /// not emitted an Outbox yet".
    pub(super) fn factory(&self) -> Option<Address> {
        self.factory
    }

    /// Whether the call that just returned completed at least one log-scan chunk. See the field's
    /// docs for why this — not a `scanned_to()` before/after diff — is the correct way for a caller
    /// to tell "failed after advancing" from "made no headway at all".
    pub(super) fn advanced_last_call(&self) -> bool {
        self.advanced_last_call
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
///
/// `factory_scan_store`, when `Some`, persists `cursor`'s progress after this call so a restart can
/// resume via [`OutboxDiscoveryCursor::from_persisted`] instead of rescanning from genesis; `None`
/// keeps discovery in-memory-only (used in tests and wherever persistence is not wired up).
pub async fn resolve<P: Provider>(
    provider: &P,
    cfg: &Config,
    destination_chain_key: B256,
    cursor: &mut OutboxDiscoveryCursor,
    factory_scan_store: Option<&FactoryScanCursorStore>,
) -> Result<Option<ResolvedOutbox>> {
    let chain_key = cfg.write_ability_chain_key;

    // The Outbox address is resolved entirely on-chain from chain_key — never configured. `cursor`
    // is the caller-owned discovery state: it advances past already-scanned *confirmed* blocks so
    // repeated resolve retries don't re-scan the whole chain history (see `resolve_outbox_address`).
    let Some((address, created_at_block)) = resolve_outbox_address(
        provider,
        chain_key,
        destination_chain_key,
        DiscoveryPolicy {
            confirmation_depth: cfg.block_confirmation_depth,
            resume_rotation_from_checkpoint: cfg.resume_rotation_from_checkpoint,
            genesis_block: cfg.factory_scan_genesis_block,
        },
        cursor,
        factory_scan_store,
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

/// Resolve the Outbox by reading the discovery-registry address off the chain-info precompile and
/// calling `defaultOutbox` on it, instead of scanning the permissionless factory's `OutboxCreated`
/// logs.
///
/// This exists for a security reason, not as a shortcut: `OutboxFactory.deployOutbox` is
/// intentionally permissionless, so any account can deploy an Outbox for `chain_key` and emit an
/// `OutboxCreated` indistinguishable from a legitimate one — [`resolve_outbox_address`]'s log scan
/// binds the newest such event, so an attacker's deployment would be permanently newest. The
/// registry (`OutboxDiscovery` in asc-contracts) is written only through an access-controlled
/// deploy path, so its `defaultOutbox` answer is safe to trust directly.
///
/// `defaultOutbox`, not `outboxOf`: confirmed as the source of truth on Slack (Kevin Nguyen,
/// 28 Aug 2026) — "the default deployed outbox for each chain via: `defaultOutbox(chainKey)` (not
/// from deployer) because there will be multiple version[s] of outbox". `outboxOf` lives on the
/// older `OutboxDeployer` and doesn't support that.
///
/// Returns `Ok(None)` — not an error — when no discovery address is registered for `chain_key` yet,
/// or the registry has no Outbox for it. The caller falls through to the factory scan in that case,
/// so a chain key not yet migrated onto the registry keeps resolving exactly as before.
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

/// Resolve the Outbox contract address for `chain_key` entirely on-chain.
///
/// 0. Prefer the discovery registry when one is registered for this chain key (see
///    [`resolve_outbox_from_registry`]) — a single trustworthy read, no scan or cursor involved.
/// 1. Otherwise, fetch the Outbox factory for this chain from the `chain-info` precompile, which
///    exposes `pallet_supported_chains::OutboxFactories` (a `chain_key → factory address` map) to
///    the EVM.
/// 2. Ask that factory for the Outbox bound to this chain key via `IOutboxFactory.getOutbox`.
///
/// Returns `Ok(None)` when no factory is registered for `chain_key` or the factory has no Outbox for
/// it yet (the on-chain values are zero). Neither address is configurable — supplying one separately
/// from the chain key is error prone, since it might not correspond to that chain key.
async fn resolve_outbox_address<P: Provider>(
    provider: &P,
    chain_key: ChainKey,
    _destination_chain_key: B256,
    policy: DiscoveryPolicy,
    cursor: &mut OutboxDiscoveryCursor,
    factory_scan_store: Option<&FactoryScanCursorStore>,
) -> Result<Option<(Address, Option<u64>)>> {
    // Reset per call, before any `.await`, so it is accurate even if this attempt is later cut off
    // by the caller's RPC_ATTEMPT_TIMEOUT — see `OutboxDiscoveryCursor::advanced_last_call`'s docs.
    cursor.advanced_last_call = false;

    // 0. Registry read, when this chain key has one — see `resolve_outbox_from_registry`. No cursor
    //    bookkeeping needed: unlike the log scan, a registry read is complete and authoritative on
    //    every call, so there is nothing to resume or persist.
    if let Some(outbox) = resolve_outbox_from_registry(provider, chain_key).await? {
        tracing::info!(
            %outbox,
            chain_key,
            "🧭 resolved Outbox on-chain (discovery registry)"
        );
        return Ok(Some((outbox, None)));
    }

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
        cursor.from = policy.genesis_block;
        cursor.scan_floor = policy.genesis_block;
        cursor.found = None;
        cursor.grace_until_tip = None;
        cursor.arm_grace_on_next_call = false;
        tracing::warn!(
            chain_key,
            "no Outbox factory registered on-chain for chain_key"
        );
        return Ok(None);
    }
    let factory = factory.factoryAddr;

    // Read the tip early, before the rotation check below: a rotation needs the *unfinalized* tip
    // observed at the moment it is detected to arm `OutboxDiscoveryCursor::grace_until_tip` against
    // (see its docs) — reading it after would race the very thing it is meant to bound against.
    //
    // Bound the scan (and thus the cursor) at the FINALIZED head, mirroring the listener's
    // `FinalityPolicy::Finalized`: Creditcoin L1 has deterministic GRANDPA finality, so a
    // finalized block cannot reorg. Only fall back to `tip - confirmation_depth` when the
    // `finalized` tag is unavailable (node up but tag unsupported/errored), matching the
    // listener's depth fallback. The cursor advances permanently, so resolving OutboxCreated
    // from a still-reorgable block would let a reorg re-mine it below the cursor and hide the
    // Outbox for the whole process lifetime — the listener signs only up to the finalized head,
    // so discovery must not run ahead of it.
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
        Ok(None) | Err(_) => tip.saturating_sub(policy.confirmation_depth),
    };

    // A cursor reloaded from disk cannot rule out that its floor is a rotation from moments before
    // the restart — arm the grace window it would have gotten live, now that a tip is in hand.
    if cursor.arm_grace_on_next_call {
        cursor.grace_until_tip = Some(tip);
        cursor.arm_grace_on_next_call = false;
    }

    // If the resolved factory changed (governance re-registered a different one for this chain key),
    // the cursor reflects the *old* factory's scanned history and could skip the new factory's
    // OutboxCreated. `cursor.from` at this point is exactly the last factory-scan checkpoint (the
    // highest block scanned before this call learned of the rotation) — resuming from there instead
    // of genesis is safe as long as the new factory is itself freshly deployed at rotation time (see
    // `Config::resume_rotation_from_checkpoint`'s docs for the assumption and its failure mode).
    if cursor.factory != Some(factory) {
        let previous_factory = cursor.factory;
        // Note the asymmetry: the resume branch keeps the checkpoint verbatim even if it sits
        // *below* `genesis_block`. The floor answers "where does a scan with no usable position
        // start?", and a checkpoint is a position; raising it here would skip range the previous
        // scan had already proven worth covering. Only the from-scratch branch takes the floor.
        let resume_from = if policy.resume_rotation_from_checkpoint {
            cursor.from
        } else {
            policy.genesis_block
        };
        if previous_factory.is_some() {
            // Not fired on the very first resolve (no `previous_factory` yet) — that is initial
            // discovery, not a rotation, and `resume_from` equals the configured genesis block
            // either way, since a fresh cursor starts there and has never scanned anything below it.
            tracing::info!(
                chain_key,
                %factory,
                ?previous_factory,
                resume_from,
                resume_rotation_from_checkpoint = policy.resume_rotation_from_checkpoint,
                genesis_block = policy.genesis_block,
                "🔁 Outbox factory rotated; restarting OutboxCreated discovery"
            );
        }
        // A fresh factory earns a fresh fallback (armed with this call's tip — see
        // `OutboxDiscoveryCursor::grace_until_tip`): this is the transition that can overshoot.
        cursor.rotate_to(factory, resume_from, tip);
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
    let sig = IOutboxFactory::OutboxCreated::SIGNATURE_HASH;
    let topic_chain_key = alloy::primitives::U256::from(chain_key);
    let mut from = cursor.from;
    // Persists whatever progress `cursor` currently reflects — called both per-chunk below and
    // after the loop (and any fallback rewind). A scan spanning many chunks in one call (the
    // initial backlog, or a genesis-fallback rescan) would otherwise checkpoint only at the very
    // end: a process restart partway through discards every chunk scanned since the last full-loop
    // completion, even though `cursor.from` itself advanced past them in memory. Best-effort — a
    // write failure only costs a re-scan on the next restart, never correctness, so it must not
    // fail resolution.
    let persist_progress = |c: &OutboxDiscoveryCursor| {
        if let Some(store) = factory_scan_store {
            let to_persist = PersistedFactoryScan {
                factory,
                scanned_to: c.from,
                found: c.found,
                scan_floor: c.scan_floor,
            };
            if let Err(err) = store.save(&to_persist) {
                tracing::warn!(chain_key, %err, "failed to persist factory-scan cursor");
            }
        }
    };
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
        // after advancing" from "made no headway" via `advanced_last_call()`. Advancing only after
        // the whole range would report zero progress for a scan that covered thousands of blocks, so
        // a long chain would burn the failure budget and restart on every attempt without ever
        // finishing discovery (bugbot). Chunks are scanned in ascending order and the cursor is the
        // resume point, so committing each completed chunk is safe: at worst the newest-Outbox scan
        // below resumes from here and re-reads nothing already covered.
        cursor.from = from;
        cursor.advanced_last_call = true;
        persist_progress(cursor);
        if from <= safe_tip {
            // More chunks remain after this one — a multi-chunk backlog (post-rotation or initial
            // activation), the same condition the relayer's own "N of M blocks" line fires under.
            // A resolve attempt that later gets cut off by the caller's RPC_ATTEMPT_TIMEOUT (a big
            // backlog easily spans several 2,000-block chunks) still leaves this line in the log for
            // every chunk it completed first — the "grinding vs wedged" ambiguity the plain 30s
            // timeout message can't resolve on its own.
            tracing::info!(
                chain_key,
                %factory,
                "🔎 scanning OutboxCreated backlog ({chunk_to} of {safe_tip} blocks); resolution not final yet"
            );
        }
    }
    // `from` is `safe_tip + 1` when the loop ran, or unchanged (`cursor.from`) if
    // `safe_tip < cursor.from` (tip regressed / depth grew) — never regressing the cursor.
    cursor.from = cursor.from.max(from);

    // Reaching here means the chunk loop ran to completion: an attempt cut short by the caller's
    // RPC_ATTEMPT_TIMEOUT is aborted mid-`await`, and a failing chunk returns via `?`. So a `found`
    // of `None` at this point is a *complete* scan of [`scan_floor`, `safe_tip`] that matched
    // nothing — which is only the same thing as "this factory has no Outbox" when the scan began at
    // the configured floor *and* the finalized tip has caught up to the height observed when that
    // floor was set (see `OutboxDiscoveryCursor::grace_until_tip`). Otherwise rewind once (or wait
    // for the grace to clear) and let the next retry cover the rest.
    let grace_satisfied = cursor.grace_until_tip.is_none_or(|until| safe_tip >= until);
    // Captured before a rewind (below) can overwrite it — the diagnostic check further down needs
    // to know what floor *this* call's scan actually ran from, not what the cursor's floor becomes
    // once a rewind reassigns it for the *next* call's scan.
    let scanned_from_this_call = cursor.scan_floor;
    if let Some(rewind_to) = genesis_fallback_target(
        cursor.found.is_some(),
        cursor.scan_floor,
        policy.genesis_block,
        grace_satisfied,
    ) {
        tracing::warn!(
            chain_key,
            %factory,
            scanned_from = cursor.scan_floor,
            scanned_to = cursor.from,
            rewind_to,
            "🪃 no OutboxCreated found above the resumed checkpoint; the factory's own event \
             may predate it (a rotation onto a pre-existing factory). Rescanning once from \
             the configured factory-scan genesis block before reporting no Outbox"
        );
        cursor.from = rewind_to;
        cursor.scan_floor = rewind_to;
    }

    // Persist whatever progress this call made, win or not (redundant with the per-chunk
    // checkpoints above whenever the loop ran, but this is also the only persist for a call that
    // found the range already scanned and skipped the loop entirely).
    //
    // Deliberately *after* the rewind above, so a fallback that has been decided is durable: a
    // restart mid-recovery resumes the rescan instead of reloading the very cursor that stranded
    // this chain key in the first place.
    persist_progress(cursor);

    if let Some((outbox, created_at_block)) = cursor.found {
        tracing::info!(
            %factory, %outbox, chain_key, ?created_at_block,
            "🧭 resolved Outbox on-chain (OutboxCreated scan)"
        );
        return Ok(Some((outbox, created_at_block)));
    }
    if policy.genesis_block > 0 && scanned_from_this_call <= policy.genesis_block {
        // This call's own scan already started at the configured floor (so no rewind fired, or
        // could have — see `scanned_from_this_call`) and still found nothing. That is the one
        // observable moment a `factory_scan_genesis_block` set above this factory's real
        // `OutboxCreated` height would produce exactly this symptom, indistinguishable otherwise
        // from "not created yet".
        tracing::warn!(
            %factory,
            chain_key,
            scanned_from = scanned_from_this_call,
            scanned_to = cursor.from,
            factory_scan_genesis_block = policy.genesis_block,
            "factory has emitted no OutboxCreated for chain_key yet, and the scan has already \
             covered everything down to factory_scan_genesis_block — if this persists, check \
             whether that floor is set above this factory's real OutboxCreated height"
        );
    } else {
        tracing::warn!(
            %factory,
            chain_key,
            scanned_from = cursor.scan_floor,
            scanned_to = cursor.from,
            "factory has emitted no OutboxCreated for chain_key yet"
        );
    }
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
            advanced_last_call: false,
            scan_floor: 0,
            grace_until_tip: None,
            arm_grace_on_next_call: false,
        };

        // Attempt 2 resumes from 1_000 and finds nothing new; the earlier discovery must survive.
        assert_eq!(cursor.scanned_to(), 1_000);
        assert_eq!(cursor.found, Some((outbox, Some(950))));

        // A factory re-registration invalidates the match: the new factory's Outbox may live
        // anywhere, and the old address must not be reported for it. This exercises
        // `resume_rotation_from_checkpoint: false` — the legacy unconditional genesis reset;
        // see `factory_rotation_resumes_from_checkpoint_when_enabled` for the default path. Goes
        // through `rotate_to`, the same method production code calls, rather than hand-copying its
        // assignments (see `the_load_bearing_rotation_reset_is_exercised_through_rotate_to`).
        let new_factory = address!("00000000000000000000000000000000000000ee");
        let resume_rotation_from_checkpoint = false;
        let resume_from = if resume_rotation_from_checkpoint {
            cursor.from
        } else {
            0
        };
        cursor.rotate_to(new_factory, resume_from, 2_000);
        assert_eq!(cursor.scanned_to(), 0);
        assert_eq!(cursor.found, None);
        assert_eq!(cursor.scan_floor, 0);
        assert_eq!(cursor.grace_until_tip, Some(2_000));
    }

    /// With `resume_rotation_from_checkpoint` (the default), a factory rotation keeps the cursor's
    /// position instead of resetting to genesis — the checkpoint is a valid floor for the new
    /// factory's own `OutboxCreated` as long as it was itself freshly deployed at rotation time
    /// (see `Config::resume_rotation_from_checkpoint`'s docs for that assumption and its failure
    /// mode). The stale `found` match is still discarded: it belongs to the old factory.
    #[test]
    fn factory_rotation_resumes_from_checkpoint_when_enabled() {
        let outbox = address!("00000000000000000000000000000000000000aa");
        let old_factory = address!("00000000000000000000000000000000000000ff");
        let new_factory = address!("00000000000000000000000000000000000000ee");

        let mut cursor = OutboxDiscoveryCursor {
            from: 500_000,
            factory: Some(old_factory),
            found: Some((outbox, Some(499_000))),
            advanced_last_call: false,
            scan_floor: 0,
            grace_until_tip: None,
            arm_grace_on_next_call: false,
        };

        let resume_rotation_from_checkpoint = true;
        let resume_from = if resume_rotation_from_checkpoint {
            cursor.from
        } else {
            0
        };
        cursor.rotate_to(new_factory, resume_from, 500_010);

        assert_eq!(cursor.scanned_to(), 500_000);
        assert_eq!(cursor.factory(), Some(new_factory));
        assert_eq!(cursor.found, None);
        assert_eq!(
            cursor.scan_floor, 500_000,
            "the resumed checkpoint becomes the new floor — this is what arms the genesis fallback"
        );
        assert_eq!(
            cursor.grace_until_tip,
            Some(500_010),
            "armed with this call's tip, so the fallback cannot fire until the finalized head \
             catches up to it"
        );
    }

    /// Pins down that the load-bearing rotation reset (`scan_floor`/`grace_until_tip`, alongside
    /// `from`/`factory`/`found`) lives in one method both production code and tests call, instead
    /// of being hand-copied at each rotation test — a hand-copy is exactly how earlier tests here
    /// drifted and stopped covering the fix this fallback exists for.
    #[test]
    fn the_load_bearing_rotation_reset_is_exercised_through_rotate_to() {
        let mut cursor = OutboxDiscoveryCursor::starting_at(0);
        cursor.factory = Some(address!("00000000000000000000000000000000000000ff"));
        cursor.from = 500_000;
        cursor.scan_floor = 0;

        cursor.rotate_to(
            address!("00000000000000000000000000000000000000ee"),
            500_000,
            500_005,
        );

        assert_eq!(
            cursor.factory(),
            Some(address!("00000000000000000000000000000000000000ee"))
        );
        assert_eq!(cursor.scanned_to(), 500_000);
        assert_eq!(cursor.scan_floor, 500_000);
        assert_eq!(cursor.found, None);
        assert_eq!(cursor.grace_until_tip, Some(500_005));
        assert!(!cursor.arm_grace_on_next_call);
    }

    /// The T4-shaped failure this fallback exists for, in cursor terms: a rotation onto a
    /// **pre-existing** factory resumes at the old factory's checkpoint, scans to the tip, and
    /// matches nothing — because the new factory's `OutboxCreated` was emitted far below. The
    /// scan is complete but not conclusive, so it rewinds to the configured floor.
    #[test]
    fn fallback_rewinds_a_resumed_scan_that_found_nothing() {
        // Resumed at 705_530, factory B's OutboxCreated is at 608_120 — never looked at. Grace is
        // satisfied (either nothing was pending, or the finalized tip has already caught up).
        assert_eq!(
            genesis_fallback_target(false, 705_530, 0, true),
            Some(0),
            "a resumed scan with no match must rewind to the floor once grace is satisfied"
        );
    }

    /// The race this fallback must not lose: a rotation onto a factory that was itself *just*
    /// deployed looks identical, for one tick, to a rotation onto a pre-existing one whose event
    /// will never be found above the checkpoint — both complete a scan and find nothing. Grace is
    /// what keeps the common, healthy rotation from burning the one-shot recovery.
    #[test]
    fn fallback_declines_while_grace_is_unsatisfied() {
        assert_eq!(
            genesis_fallback_target(false, 705_530, 0, false),
            None,
            "grace not yet satisfied — the fresh factory's own event may simply not have finalized"
        );
    }

    /// The two ways the fallback correctly declines when grace isn't the blocker.
    #[test]
    fn fallback_declines_when_the_scan_is_already_conclusive() {
        assert_eq!(
            genesis_fallback_target(true, 705_530, 0, true),
            None,
            "a match makes the scan conclusive whatever floor it began at"
        );
        assert_eq!(
            genesis_fallback_target(false, 0, 0, true),
            None,
            "a scan that began at the floor has covered everything; no Outbox is the truth"
        );
    }

    /// The floor is honoured, not hardcoded to 0: with `factory_scan_genesis_block` set, the rewind
    /// targets it, and a scan that already started there is conclusive rather than looping.
    #[test]
    fn fallback_targets_the_configured_genesis_block() {
        assert_eq!(
            genesis_fallback_target(false, 705_530, 300_000, true),
            Some(300_000)
        );
        assert_eq!(
            genesis_fallback_target(false, 300_000, 300_000, true),
            None,
            "starting at the configured floor is as complete as a scan can be"
        );
        assert_eq!(
            genesis_fallback_target(false, 250_000, 300_000, true),
            None,
            "already below the floor: nothing above it went unscanned"
        );
    }

    /// The fallback terminates without a separate "already used" flag: once it fires, the caller
    /// lowers `scan_floor` to `genesis_block`, and — `genesis_block` being constant per process —
    /// every later call finds `scan_floor > genesis_block` already false. The floor alone carries
    /// the termination invariant, walked here through the full recovery: rotate, scan, no match,
    /// rewind, rescan, still no match, must end in a settled state rather than a rescan loop.
    #[test]
    fn fallback_terminates_after_one_rewind() {
        let mut cursor = OutboxDiscoveryCursor::starting_at(0);
        cursor.factory = Some(address!("00000000000000000000000000000000000000ff"));
        cursor.from = 705_530;
        cursor.scan_floor = 705_530;

        // First completion: nothing found above the resumed floor, grace satisfied → rewind.
        let first = genesis_fallback_target(cursor.found.is_some(), cursor.scan_floor, 0, true);
        assert_eq!(first, Some(0));
        cursor.from = 0;
        cursor.scan_floor = 0;

        // Second completion after the full rescan: still nothing. The floor alone now says stop.
        cursor.from = 705_600;
        assert_eq!(
            genesis_fallback_target(cursor.found.is_some(), cursor.scan_floor, 0, true),
            None
        );
    }

    /// The spurious-fire race this fallback must not lose to, worked through with the actual tip
    /// arithmetic `resolve_outbox_address` uses: a rotation observes tip 500_005, and the very same
    /// tick's scan is bounded by a *finalized* `safe_tip` that lags behind it — so an empty result
    /// on that tick must not be trusted, only once the finalized head has caught up to 500_005.
    #[test]
    fn grace_declines_immediately_after_rotation_and_clears_once_finality_catches_up() {
        let mut cursor = OutboxDiscoveryCursor::starting_at(0);
        cursor.rotate_to(
            address!("00000000000000000000000000000000000000ee"),
            500_000, // old checkpoint, resumed
            500_005, // unfinalized tip observed at the moment the rotation was detected
        );

        // Same tick: the scan completes [500_000, 500_003] and finds nothing — the new factory's
        // own event, even if emitted around rotation time, may simply not have finalized yet.
        let safe_tip_same_tick = 500_003;
        let grace_satisfied = cursor
            .grace_until_tip
            .is_none_or(|until| safe_tip_same_tick >= until);
        assert!(!grace_satisfied);
        assert_eq!(
            genesis_fallback_target(false, cursor.scan_floor, 0, grace_satisfied),
            None,
            "must not burn the one-shot recovery on a rotation that is merely awaiting finality"
        );

        // Later tick: the finalized head has now passed the rotation-observed tip, so anything at
        // or before it — including a same-tick `OutboxCreated` — has had its chance to finalize.
        let safe_tip_later = 500_006;
        let grace_satisfied = cursor
            .grace_until_tip
            .is_none_or(|until| safe_tip_later >= until);
        assert!(grace_satisfied);
        assert_eq!(
            genesis_fallback_target(false, cursor.scan_floor, 0, grace_satisfied),
            Some(0),
            "still no match once finality has caught up — now it is evidence, not a race"
        );
    }

    /// `starting_at` is what a fresh cursor uses when nothing is persisted: the configured floor is
    /// both the scan position and the floor, so a first scan is immediately conclusive, and there
    /// is no pending grace to wait on.
    #[test]
    fn starting_at_seeds_position_and_floor_together() {
        let cursor = OutboxDiscoveryCursor::starting_at(300_000);
        assert_eq!(cursor.scanned_to(), 300_000);
        assert_eq!(cursor.scan_floor, 300_000);
        assert_eq!(cursor.factory(), None);
        assert_eq!(cursor.grace_until_tip, None);
        assert!(!cursor.arm_grace_on_next_call);
        assert_eq!(
            genesis_fallback_target(false, cursor.scan_floor, 300_000, true),
            None
        );
    }

    /// A cursor reloaded from disk carries the floor its scan actually began at, so a restart in
    /// the middle of the stranded state still recognises it as unresolved rather than trusting the
    /// tip-height checkpoint as a completed scan. This is what makes the recovery survive the
    /// "restart it and see" reflex.
    #[test]
    fn from_persisted_carries_the_floor_so_a_restart_can_still_recover() {
        let persisted = PersistedFactoryScan {
            factory: address!("00000000000000000000000000000000000000ee"),
            scanned_to: 705_600,
            found: None,
            scan_floor: 705_530,
        };
        let cursor = OutboxDiscoveryCursor::from_persisted(persisted);
        assert_eq!(cursor.scanned_to(), 705_600);
        assert_eq!(cursor.scan_floor, 705_530);
        assert!(
            cursor.arm_grace_on_next_call,
            "a reload can't rule out the floor being a rotation from moments before the restart, \
             so it defers arming the grace window to the first live call instead of trusting an \
             empty scan blind"
        );
        assert_eq!(
            genesis_fallback_target(false, cursor.scan_floor, 0, true),
            Some(0),
            "the decision itself is unaffected by the reload — only the caller's grace check is"
        );
    }

    /// Regression (bugbot): a caller that diffs `scanned_to()` before and after a call to tell
    /// "advanced" from "no headway" gets it backwards when `resume_rotation_from_checkpoint: false`
    /// resets `from` to genesis *inside* that same call — the post-call value can land below the
    /// pre-call snapshot even though the fresh scan clearly advanced. `advanced_last_call()` reports
    /// per-call progress directly instead, independent of where `from` ends up.
    #[test]
    fn advanced_last_call_survives_a_rotation_reset_within_the_same_call() {
        let mut cursor = OutboxDiscoveryCursor {
            from: 500_000,
            factory: Some(address!("00000000000000000000000000000000000000ff")),
            found: None,
            advanced_last_call: false,
            scan_floor: 0,
            grace_until_tip: None,
            arm_grace_on_next_call: false,
        };
        let scanned_before = cursor.scanned_to();

        // What `resolve_outbox_address` does within a single call: reset at the top, then a
        // rotation with `resume_rotation_from_checkpoint: false` sets `from` back to 0, then one
        // chunk of the fresh scan completes.
        cursor.advanced_last_call = false;
        cursor.from = 0;
        cursor.from = 2_000;
        cursor.advanced_last_call = true;

        assert!(
            cursor.scanned_to() < scanned_before,
            "the old factory's checkpoint is not a valid baseline for the new factory's scan"
        );
        assert!(
            cursor.advanced_last_call(),
            "a chunk was scanned this call, regardless of where `from` ended up"
        );
    }

    /// A cursor seeded from a persisted scan carries over its cursor position, factory and winner
    /// verbatim — the whole point of persisting it is to skip re-scanning that range.
    #[test]
    fn from_persisted_seeds_cursor_verbatim() {
        let outbox = address!("00000000000000000000000000000000000000aa");
        let factory = address!("00000000000000000000000000000000000000ff");
        let persisted = PersistedFactoryScan {
            factory,
            scanned_to: 12_345,
            found: Some((outbox, Some(950))),
            scan_floor: 0,
        };
        let cursor = OutboxDiscoveryCursor::from_persisted(persisted);
        assert_eq!(cursor.scanned_to(), 12_345);
        assert_eq!(cursor.factory(), Some(factory));
        assert_eq!(cursor.found, Some((outbox, Some(950))));
    }

    /// A seed recorded against a factory other than the one currently registered on-chain must not
    /// be reused verbatim — `resolve_outbox_address`'s ordinary `cursor.factory != factory`
    /// comparison is what actually discards it, but this pins down that `from_persisted` itself
    /// does no filtering, so that comparison is the only thing standing between a stale seed and a
    /// wrongly-carried-over cursor.
    #[test]
    fn from_persisted_does_not_filter_by_factory_itself() {
        let persisted = PersistedFactoryScan {
            factory: address!("00000000000000000000000000000000000000ff"),
            scanned_to: 12_345,
            found: None,
            scan_floor: 0,
        };
        let cursor = OutboxDiscoveryCursor::from_persisted(persisted);
        let live_factory = address!("00000000000000000000000000000000000000ee");
        assert_ne!(cursor.factory(), Some(live_factory));
    }
}
