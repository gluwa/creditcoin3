//! USC write-ability: cross-chain message attestation (confluence §7.3).
//!
//! When enabled (`message_attestation_enabled`), this task makes the attestor a **message
//! validator**: it watches the Creditcoin L1 Outbox for its `chain_key`, signs the canonical
//! `messageHash` of each finalized `MessagePublished`, and gossips an ECDSA [`MessageVote`] on
//! `{chain_key}/message-votes/v1`. Relayers snoop the same topic and deliver once 2/3+1 unique
//! attestors have voted — the attestor never relays or touches the destination chain (§1).
//!
//! **Transport reuse:** message votes ride the *existing* attestor libp2p swarm — same peers,
//! discovery (kad/mdns/identify), and bootnodes — adding only the new topic. This task therefore
//! owns no swarm: it produces votes and hands them to the [`p2p`](crate::tasks::p2p) task to
//! publish, and shares the [`VoteAggregator`] + active set with it via [`MessageVoteState`] on
//! [`Shared`]. Incoming peer votes are validated + counted inline by the p2p task through
//! [`ingest::validate_and_count`].
//!
//! Pipeline: [`resolver`] → [`listener`] (finality-gated `MessagePublished`) → [`signing`] →
//! count locally + publish; peers' votes → [`ingest`] → [`aggregator`].
//!
//! [`MessageVote`]: write_ability::envelope::MessageVote
//! [`Shared`]: crate::shared::Shared
//! [`VoteAggregator`]: aggregator::VoteAggregator

pub mod aggregator;
pub mod attestor_set;
pub mod config;
pub mod cursor;
pub mod ingest;
pub mod listener;
pub mod reobservation;
pub mod resolver;
pub mod set_update;
pub mod signing;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alloy::primitives::{Address, B256};
use alloy::providers::{Provider, ProviderBuilder};
use anyhow::anyhow;
use parking_lot::{Mutex, RwLock};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

use write_ability::envelope::{MessageVote, ReobservationRequest, SetUpdateVote};
use write_ability::protocol::chain_key_to_bytes32;

use crate::error::Error;
use crate::shared::Shared;

pub use config::{AttestorSet, Config};

/// How often to re-attempt Outbox resolution while it is not yet registered on-chain (dynamic
/// activation without a restart).
const OUTBOX_RESOLVE_RETRY_SECS: u64 = 12;

/// Wall-clock bound for one write-ability RPC attempt. Every long-lived loop catches this error and
/// retries with a fresh attempt; startup-only calls surface it to the process supervisor so a
/// black-holed socket can never leave the pod serving a permanently-green health endpoint.
pub(super) const RPC_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(30);

/// After this many consecutive failed resolves (~5 min at [`OUTBOX_RESOLVE_RETRY_SECS`]) the retry is
/// probably no longer "waiting for on-chain registration" but a misconfiguration — most likely a
/// deploy-ordering trap where the attestor was upgraded ahead of the runtime, so the renamed
/// chain-info selector (`get_outbox_discovery_address`) reverts and resolution can never succeed (S3).
/// Escalate the log to error-level at each multiple so it is alertable instead of buried in warns.
const RESOLVE_ESCALATE_EVERY_ATTEMPTS: u64 = (5 * 60) / OUTBOX_RESOLVE_RETRY_SECS;

/// A responsive node may legitimately report that no Outbox exists yet (`Ok(None)`) forever, but
/// repeated RPC errors against the same bare alloy provider mean the connection is no longer
/// usable. After this many consecutive failures, rebuild the provider in place — see
/// [`connect_l1_provider`] — rather than retrying the same dead socket indefinitely.
///
/// This used to return an error instead, on the theory that a process supervisor would rebuild the
/// provider by restarting us. That only holds where a supervisor exists: under zombienet nothing
/// restarts the attestor, so a long enough outage killed it permanently and took every other
/// (healthy) task down with it — p2p, validation, production and api all exited cleanly behind a
/// resolver that could not reach the RPC. Rebuilding in place removes the dependency on an
/// external supervisor and keeps an RPC outage scoped to the one task that cares about it.
///
/// Sized to ~5 minutes (matching [`RESOLVE_ESCALATE_EVERY_ATTEMPTS`]): a *planned* node restart —
/// the CI outage-recovery scenarios bounce the CC3 node for a couple of minutes, and a devnet
/// rollout looks the same — rides out on plain retries well below this. Crossing it means the
/// provider itself is suspect, so it buys a fresh connection, not a dead process.
const MAX_CONSECUTIVE_RESOLVE_FAILURES: u64 = (5 * 60) / OUTBOX_RESOLVE_RETRY_SECS;

/// Message-vote state shared between this task (producer) and the p2p task (publisher + incoming
/// validator). Lives on [`Shared`](crate::shared::Shared) as `Option`, set only when message
/// attestation is enabled with a usable attestor set.
pub struct MessageVoteState {
    /// In-memory vote aggregator (chain-first allowlist, dedup, threshold, anti-abuse caps).
    pub aggregator: Mutex<aggregator::VoteAggregator>,
    /// Authorized signer EVM addresses; gossip votes from outside this set are rejected. Behind a
    /// lock so the [`attestor_set`] watcher can hot-swap it (with the aggregator threshold) when the
    /// on-chain `EOAValidator` set changes — no restart needed. Reads (one per incoming vote)
    /// dominate writes (rare set changes), hence `RwLock`.
    pub active_set: RwLock<HashSet<Address>>,
    /// Outgoing votes we produced, handed to the p2p task to publish on the message-vote topic.
    pub publish_tx: mpsc::Sender<MessageVote>,
    /// Outgoing attestor-set-update votes (P2-8), handed to the p2p task to publish on the
    /// set-update topic. Set by [`build_state`]; drained by the p2p task.
    pub set_update_publish_tx: mpsc::Sender<SetUpdateVote>,
    /// Incoming reobservation requests the p2p task decoded off the reobservation topic, handed to
    /// the write-ability task to verify + re-sign. `try_send` from the swarm loop (best effort:
    /// shedding a request under a full buffer just means that stall recovers on the next request).
    pub reobs_tx: mpsc::Sender<ReobservationRequest>,
    /// The `bytes32` write-ability chain key bound into every `messageHash` and used to resolve the
    /// Outbox via `historical OutboxDiscovery membership`. Sourced from the on-chain `WriteAbilityConfigs`
    /// entry when one is registered for this `chain_key`; derived locally (right-padded `u64`)
    /// otherwise. `None` pauses signing until a successful finalized governance read enables it.
    /// Held through signing so a concurrent disable/key change cannot authorize a stale message.
    pub destination_chain_key: RwLock<Option<B256>>,
}

/// Build the shared message-vote state and the matching publish channel receiver from config, or
/// `None` when message attestation is disabled / not yet supported. Runs during `lib.rs` startup
/// before tasks spawn; resolving an [`AttestorSet::OnChainValidator`] set performs one RPC read.
///
/// Enablement is gated twice: the local `enabled` flag is the *operator's* opt-in (and implies the
/// RPC endpoints are configured), while the on-chain `WriteAbilityConfigs` entry for `chain_key` is
/// *governance's* switch — when an entry exists with `message_attestation_enabled == false` the
/// task stays off regardless of local config. A successfully read missing entry falls back to
/// local config for dev setups. Signing starts paused; after a successful read, failed refreshes
/// retain the last known authorization. The task remains alive while disabled
/// so finalized governance changes can enable it or change its destination key without a restart.
#[must_use]
pub async fn build_state(
    cfg: &Config,
    cc3: &cc_client::Client,
) -> Option<(
    Arc<MessageVoteState>,
    mpsc::Receiver<MessageVote>,
    mpsc::Receiver<ReobservationRequest>,
    mpsc::Receiver<SetUpdateVote>,
)> {
    if !cfg.enabled {
        return None;
    }
    // Only nag operators who actually configured it: the field defaults to a non-zero value, so
    // an unconditional warning would fire on every attestor that never mentioned the setting.
    if cfg.block_confirmation_depth != config::DEFAULT_BLOCK_CONFIRMATION_DEPTH {
        tracing::warn!(
            block_confirmation_depth = cfg.block_confirmation_depth,
            "block_confirmation_depth is deprecated and ignored for message attestation; \
             source blocks must be finalized"
        );
    }
    // Fail-closed runtime-compatibility gate (audit P2-9). The write-ability chain state
    // (`WriteAbilityConfigs`, the `SupportedChainsApi` v2 methods) only exists on a
    // write-ability-capable runtime. Against a pre-write-ability (v1) runtime the on-chain reads
    // return nothing and the attestor would otherwise silently fall back to a locally-derived chain
    // key — which may diverge from on-chain governance. Refuse to enable message attestation loudly
    // instead, so a runtime/attestor version skew is an operator-visible error, not a silent
    // mis-signing. (The attestor's other duties are unaffected — only this task disables.)
    if !cc3.supports_write_ability() {
        tracing::error!(
            "❌ message attestation is enabled but the connected Creditcoin runtime does not \
             support write-ability (SupportedChainsApi < v2 / no WriteAbilityConfigs) — refusing \
             to enable message attestation. Upgrade the runtime or disable message attestation."
        );
        return None;
    }
    let active_set = resolve_active_set(cfg).await?;
    let threshold = attestor_primitives::calculate_threshold(active_set.len() as u32) as usize;
    let aggregator =
        aggregator::VoteAggregator::new(threshold, cfg.max_tracked_messages, cfg.vote_ttl);
    let (publish_tx, publish_rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let (reobs_tx, reobs_rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let (set_update_publish_tx, set_update_publish_rx) =
        mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let state = Arc::new(MessageVoteState {
        aggregator: Mutex::new(aggregator),
        active_set: RwLock::new(active_set),
        publish_tx,
        set_update_publish_tx,
        reobs_tx,
        destination_chain_key: RwLock::new(None),
    });
    tracing::info!(
        attestors = state.active_set.read().len(),
        threshold,
        "🧑‍🤝‍🧑 message-vote quorum configured"
    );
    Some((state, publish_rx, reobs_rx, set_update_publish_rx))
}

/// Register this attestor's write-ability EVM message-vote address on-chain (audit P2-8),
/// idempotently and fully best-effort. Returns `true` once the address is confirmed on-chain (or was
/// already), `false` on any failure this attempt (so a caller can retry).
///
/// Proves possession over the pallet's registration digest (`cc3.evm_registration_digest`) with the
/// attestor's EVM `signer` and submits `set_attestor_evm_address` only when the on-chain value is
/// missing or differs — so a restart is a no-op. `chain_key` is the attestation chain key the
/// attestor is registered under (the pallet keys the EVM address the same way). **Never fatal**:
/// every failure (including sign) is logged and returned as `false`; the attestor keeps attesting and
/// the destination `EOAValidator` set omits it until a later attempt succeeds. Requires the attestor
/// to already be registered on-chain (the pallet rejects a non-attestor), so call it after `attest`.
pub async fn register_evm_address(
    cc3: &cc_client::Client,
    signer: &signing::MessageSigner,
    chain_key: attestor_primitives::ChainKey,
) -> bool {
    let address = signer.address();
    let ours = sp_core::H160::from_slice(address.as_slice());

    let existing =
        tokio::time::timeout(RPC_ATTEMPT_TIMEOUT, cc3.attestor_evm_address(chain_key)).await;
    match existing {
        Ok(Ok(Some(existing))) if existing == ours => {
            tracing::debug!(evm_address = %address, "🔗 write-ability EVM address already registered on-chain");
            return true;
        }
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            tracing::warn!(%err, "could not read the on-chain EVM address registration; will attempt to (re)register");
        }
        Err(_) => {
            tracing::warn!(
                timeout_secs = RPC_ATTEMPT_TIMEOUT.as_secs(),
                "timed out reading the on-chain EVM address registration; will attempt to (re)register"
            );
        }
    }

    let digest = cc3.evm_registration_digest(chain_key);
    let proof = match signer.sign(&B256::from(digest)) {
        Ok(p) => p,
        Err(err) => {
            tracing::error!(%err, "could not sign EVM registration digest — cannot register");
            return false;
        }
    };

    match tokio::time::timeout(
        RPC_ATTEMPT_TIMEOUT,
        cc3.set_attestor_evm_address(chain_key, ours, proof),
    )
    .await
    {
        Ok(Ok(())) => {
            tracing::info!(evm_address = %address, "🔗 registered write-ability EVM address on-chain");
            true
        }
        Ok(Err(err)) => {
            tracing::warn!(
                %err,
                evm_address = %address,
                "failed to register write-ability EVM address on-chain — will retry; the destination EOAValidator set omits this attestor until it succeeds"
            );
            false
        }
        Err(_) => {
            tracing::warn!(
                evm_address = %address,
                timeout_secs = RPC_ATTEMPT_TIMEOUT.as_secs(),
                "timed out registering write-ability EVM address on-chain — will retry"
            );
            false
        }
    }
}

/// Read the on-chain `WriteAbilityConfigs` entry for this `chain_key` and derive the effective
/// `bytes32` write-ability chain key. `Ok(None)` means governance disabled attestation; an error
/// does not change the last known authorization. Only a successful missing-entry response uses
/// local configuration.
async fn resolve_destination_chain_key(
    cfg: &Config,
    cc3: &cc_client::Client,
) -> anyhow::Result<Option<B256>> {
    read_governance(
        cfg.write_ability_chain_key,
        cc3.get_write_ability_config(cfg.write_ability_chain_key),
        RPC_ATTEMPT_TIMEOUT,
    )
    .await
}

async fn read_governance<E: std::fmt::Display>(
    chain_key: u64,
    read: impl std::future::Future<
        Output = Result<Option<supported_chains_primitives::WriteAbilityConfig>, E>,
    >,
    timeout: Duration,
) -> anyhow::Result<Option<B256>> {
    let local = chain_key_to_bytes32(chain_key);
    let config = tokio::time::timeout(timeout, read).await;
    match config {
        Ok(Ok(Some(on_chain))) => {
            if !on_chain.message_attestation_enabled {
                tracing::info!(
                    chain_key,
                    "📴 on-chain WriteAbilityConfig disables message attestation for this chain — disabling"
                );
                return Ok(None);
            }
            let key = B256::from(on_chain.write_ability_chain_key);
            if key != local {
                tracing::warn!(
                    chain_key,
                    on_chain_key = %key,
                    derived_key = %local,
                    "on-chain write-ability chain key differs from the locally derived one — using the on-chain value"
                );
            }
            Ok(Some(key))
        }
        Ok(Ok(None)) => {
            tracing::warn!(
                chain_key,
                "no on-chain WriteAbilityConfig registered for this chain — using the locally derived chain key"
            );
            Ok(Some(local))
        }
        Ok(Err(err)) => Err(anyhow!("read finalized WriteAbilityConfig: {err}")),
        Err(_) => Err(anyhow!(
            "read finalized WriteAbilityConfig timed out after {timeout:?}"
        )),
    }
}

/// Only authoritative responses change authorization. On an RPC failure the initial `None`
/// stays paused, while a previously enabled or disabled configuration remains in force. Keeping
/// the error distinct from `Ok(None)` also prevents the monitor from aborting the listener and
/// discarding its in-flight work. Degradation is observable but never a liveness restart signal.
fn apply_governance_read(
    state: &MessageVoteState,
    metrics: &metrics::Metrics,
    result: anyhow::Result<Option<B256>>,
) -> anyhow::Result<Option<B256>> {
    match result {
        Ok(key) => {
            apply_governance(state, key);
            if metrics.set_write_ability_governance_degraded(false) {
                tracing::info!("finalized governance reads recovered");
            }
            Ok(key)
        }
        Err(err) => {
            metrics.set_write_ability_governance_degraded(true);
            tracing::warn!(
                error = %format!("{err:#}"),
                destination_chain_key = ?*state.destination_chain_key.read(),
                "governance refresh degraded — retaining current signing authorization; startup stays paused until the first successful read"
            );
            Err(err)
        }
    }
}

/// Publish governance before updating routing. Clearing the chain-seen vote cache prevents a
/// previous destination's votes being accepted after a disable or key change. Lock order matches
/// `produce_vote`: governance first, then aggregator.
fn apply_governance(state: &MessageVoteState, next: Option<B256>) {
    let mut current = state.destination_chain_key.write();
    if *current != next {
        state.aggregator.lock().clear_messages();
        *current = next;
        tracing::info!(destination_chain_key = ?next, "message-attestation governance updated");
    }
}

/// Both activation and the live monitor use the same finalized governance read. Subxt's
/// `storage().at_latest()` in `get_write_ability_config` selects the finalized head. Each read has
/// its own deadline. A failed refresh preserves authorization and routing until a successful read.
async fn resolve_authorized_outbox<P: Provider>(
    provider: &P,
    cfg: &Config,
    cc3: &cc_client::Client,
    state: &MessageVoteState,
    metrics: &metrics::Metrics,
) -> anyhow::Result<Option<resolver::ResolvedRoute>> {
    let result = resolve_destination_chain_key(cfg, cc3).await;
    let key = apply_governance_read(state, metrics, result)?;
    let Some(key) = key else { return Ok(None) };
    tokio::time::timeout(
        RPC_ATTEMPT_TIMEOUT,
        resolver::resolve(provider, cfg.write_ability_chain_key, key),
    )
    .await
    .map_err(|_| anyhow!("Outbox resolution timed out after {RPC_ATTEMPT_TIMEOUT:?}"))?
}

/// Resolve the authorized signer set. Returns `None` (with a logged reason) when the set can't be
/// determined, which disables message attestation for the run while the rest of the attestor keeps
/// working.
///
/// * [`AttestorSet::Static`] — the configured address list.
/// * [`AttestorSet::OnChainValidator`] — read `EOAValidator.attestors()` from the **destination
///   chain** (the chain this attestor set attests, where the validator lives), via
///   `destination_eth_rpc_url`. This is the on-chain source of truth, kept in sync with the Inbox.
async fn resolve_active_set(cfg: &Config) -> Option<HashSet<Address>> {
    match &cfg.attestor_set {
        AttestorSet::Static(addrs) if !addrs.is_empty() => Some(addrs.iter().copied().collect()),
        AttestorSet::Static(_) => {
            tracing::error!("message attestation enabled but attestor_set is empty — disabling");
            None
        }
        AttestorSet::OnChainValidator(validator) => {
            if cfg.destination_eth_rpc_url.is_none() {
                tracing::error!(
                    %validator,
                    "OnChainValidator attestor set configured but no destination_eth_rpc_url — disabling"
                );
                return None;
            }
            // Do NOT block startup on the destination RPC. Start with an empty set — ingest rejects
            // every vote (signer ∉ set) until it is populated — and let the `attestor_set::watch`
            // task, spawned by `run` *off the core-startup path*, fill the set + aggregator threshold
            // on its first tick (which fires immediately) and re-poll every `ATTESTOR_SET_POLL_SECS`.
            // Previously this fetched here with ~94s of bounded retry inside `build_state`, delaying
            // block-attestation / p2p / production startup (Bugbot: "Write-ability blocks attestor
            // startup"). The watcher's per-poll retry also recovers from a startup RPC blip without a
            // restart, which supersedes the old bounded-retry (C2) rationale.
            tracing::info!(
                %validator,
                "🧑‍⚖️ on-chain attestor set will be populated by the watcher (non-blocking startup)"
            );
            Some(HashSet::new())
        }
    }
}

/// Connect the Creditcoin L1 EVM provider used for Outbox discovery and message observation.
///
/// Factored out of [`run`] so the resolve loop can rebuild the connection in place once it looks
/// wedged (see [`MAX_CONSECUTIVE_RESOLVE_FAILURES`]) instead of returning an error and relying on
/// a process supervisor to do it. Yields `anyhow::Error` rather than [`Error`] because the rebuild
/// path logs and carries on; only the initial call in `run` treats a failure as fatal, and there a
/// provider we could never build at all genuinely is.
async fn connect_l1_provider(rpc: &str) -> anyhow::Result<impl Provider + Clone + 'static> {
    tokio::time::timeout(RPC_ATTEMPT_TIMEOUT, ProviderBuilder::new().on_builtin(rpc))
        .await
        .map_err(|_| {
            anyhow!("connect Creditcoin L1 EVM RPC timed out after {RPC_ATTEMPT_TIMEOUT:?}")
        })?
        .map_err(|e| anyhow!("connect Creditcoin L1 EVM RPC: {e}"))
}

/// Entry point spawned from `lib.rs`. Drives the Outbox listener and produces signed votes; the
/// swarm itself is owned by the p2p task. `seed` is the 32-byte secret the EVM key derives from.
/// `reobs_rx` carries reobservation requests the p2p task decoded off the reobservation topic.
pub async fn run(
    shared: Arc<Shared>,
    cfg: Config,
    seed: Zeroizing<[u8; 32]>,
    reobs_rx: mpsc::Receiver<ReobservationRequest>,
    cc3: Arc<cc_client::Client>,
) -> Result<(), Error> {
    let Some(state) = shared.message_votes.clone() else {
        tracing::info!("📭 message attestation disabled — parking write-ability task");
        // Park until shutdown; returning Ok early would trip the supervisor's "exited early" guard.
        shared.token.cancelled().await;
        return Ok(());
    };

    // Durable storage is mandatory for a write-ability attestor. Verify the state directory is
    // writable up front so a missing / read-only volume fails the boot loudly here, rather than
    // silently degrading to no cursor persistence (the restart-loses-messages footgun) once the
    // listener is running.
    cursor::ensure_writable(&cfg.state_dir).map_err(Error::WriteAbility)?;

    // On-chain attestor-set hot-reload watcher (only when the set is sourced from the validator).
    // Runs independently of Outbox resolution — the set is unrelated to the Outbox — so it keeps the
    // active set in sync even while write-ability is idle waiting for the Outbox.
    let mut set_watcher = match (&cfg.attestor_set, cfg.destination_eth_rpc_url.as_ref()) {
        (AttestorSet::OnChainValidator(validator), Some(url)) => {
            Some(tokio::spawn(attestor_set::watch(
                state.clone(),
                *validator,
                url.to_string(),
                shared.token.clone(),
            )))
        }
        (AttestorSet::OnChainValidator(validator), None) => {
            tracing::warn!(
                %validator,
                "OnChainValidator set but no destination_eth_rpc_url — attestor set will not hot-reload"
            );
            None
        }
        _ => None,
    };

    // Attestor-set-update proposer (P2-8): gossips a signed vote when the elected set (from the
    // on-chain EVM-address registry) diverges from the destination validator's set, for the relayer
    // to submit. Like `set_watcher` it is Outbox-independent, only meaningful for an OnChainValidator
    // set, and derives its own EVM signer from `seed`.
    let mut set_update_proposer = match (&cfg.attestor_set, cfg.destination_eth_rpc_url.as_ref()) {
        (AttestorSet::OnChainValidator(validator), Some(url)) => {
            match signing::MessageSigner::from_seed(&seed) {
                Ok(proposer_signer) => Some(tokio::spawn(set_update::run_proposer(
                    cc3.clone(),
                    cfg.write_ability_chain_key,
                    url.to_string(),
                    *validator,
                    proposer_signer,
                    state.set_update_publish_tx.clone(),
                    shared.token.clone(),
                ))),
                Err(err) => {
                    tracing::error!(%err, "could not derive EVM signer for set-update proposer — disabling it");
                    None
                }
            }
        }
        _ => None,
    };

    let rpc = cfg.cc3_eth_rpc_url.as_ref().ok_or_else(|| {
        Error::WriteAbility(anyhow!(
            "cc3_eth_rpc_url is required when message attestation is on"
        ))
    })?;
    // `mut` so the resolve loop can swap in a fresh connection when the current one wedges. Safe
    // to reassign here: the first clone handed to a spawned task is taken well after the loop.
    let mut provider = connect_l1_provider(rpc.as_str())
        .await
        .map_err(Error::WriteAbility)?;

    // Capture the chain head *before* the resolve loop. When no explicit `start_block` is
    // configured we scan from here, not from the head after resolution finishes — otherwise
    // messages published on the Outbox during the resolve-retry window (the Outbox can exist and
    // receive messages a poll interval before we resolve it) would be silently skipped, never
    // signed or gossiped. Operators expecting a long activation wait should still set `start_block`
    // to bound the initial backfill range (no MessagePublished logs exist before Outbox creation,
    // so the effective scan is small in the common case).
    let head_before_resolve =
        tokio::time::timeout(RPC_ATTEMPT_TIMEOUT, provider.get_block_number())
            .await
            .map_err(|_| {
                Error::WriteAbility(anyhow!(
                    "read Creditcoin L1 chain head timed out after {RPC_ATTEMPT_TIMEOUT:?}"
                ))
            })?
            .map_err(|e| Error::WriteAbility(anyhow!("read Creditcoin L1 chain head: {e}")))?;

    // Resolve the Outbox, retrying until it's available rather than disabling for the whole run:
    // an attestor can be started before a discovery address is registered on-chain for its chain
    // key and will activate write-ability automatically once one is, with no restart. While
    // unresolved it just keeps doing block attestation. Resolution stays live after activation in
    // `run_outbox_monitor`, which detects registry changes with the same polling.
    let mut resolve_attempts: u64 = 0;
    let mut consecutive_resolve_failures: u64 = 0;
    let resolved = loop {
        let attempt = tokio::select! {
            attempt = resolve_authorized_outbox(&provider, &cfg, &cc3, &state, &shared.metrics) => attempt,
            joined = wait_for_optional_child(&mut set_watcher) => {
                if let Some(proposer) = &set_update_proposer {
                    proposer.abort();
                }
                // On shutdown these children exit cleanly with `Ok(())` and race the cancel branch,
                // so this arm winning must not turn an intentional stop into a task failure. The
                // post-activation loop already guards the same way.
                if shared.token.is_cancelled() {
                    return Ok(());
                }
                return Err(Error::WriteAbility(child_exit_error("attestor-set watcher", joined)));
            }
            joined = wait_for_optional_child(&mut set_update_proposer) => {
                if let Some(watcher) = &set_watcher {
                    watcher.abort();
                }
                // See the watcher arm: a clean child exit during shutdown must not be reported as a
                // failure.
                if shared.token.is_cancelled() {
                    return Ok(());
                }
                return Err(Error::WriteAbility(child_exit_error("attestor-set-update proposer", joined)));
            }
            () = shared.token.cancelled() => {
                if let Some(watcher) = &set_watcher {
                    watcher.abort();
                }
                if let Some(proposer) = &set_update_proposer {
                    proposer.abort();
                }
                return Ok(());
            }
        };
        match attempt {
            Ok(Some(r)) => break r,
            Ok(None) => {
                consecutive_resolve_failures = 0;
                resolve_attempts += 1;
                if resolve_attempts % RESOLVE_ESCALATE_EVERY_ATTEMPTS == 0 {
                    // WARN, not ERROR: `Ok(None)` means nothing is registered on-chain yet, which is
                    // a benign, expected state — per the retry loop above, the attestor keeps doing
                    // block attestation and activates write-ability by itself once a factory/Outbox
                    // appears. Whole environments (e.g. the attestor-network integration tests) never
                    // configure write-ability at all, so escalating to ERROR reported "broken" for
                    // "not configured" and tripped the CI attestor error gate. A resolve that is
                    // actually *failing* is a different case and still escalates to ERROR below.
                    tracing::warn!(
                        attempts = resolve_attempts,
                        elapsed_secs = resolve_attempts * OUTBOX_RESOLVE_RETRY_SECS,
                        "⏳ Outbox still unresolved after prolonged retrying — if this chain is meant to serve write-ability, verify the on-chain WriteAbilityConfigs entry and the runtime/attestor deploy ordering (chain-info `get_outbox_discovery_address` selector)"
                    );
                } else {
                    tracing::info!(
                        retry_secs = OUTBOX_RESOLVE_RETRY_SECS,
                        "📭 no Outbox factory/Outbox registered on-chain yet — write-ability idle; will retry"
                    );
                }
            }
            Err(err) => {
                resolve_attempts += 1;
                consecutive_resolve_failures += 1;
                if consecutive_resolve_failures >= MAX_CONSECUTIVE_RESOLVE_FAILURES {
                    // Reset the window before the attempt, not after it: whether the rebuild
                    // succeeds or not we want the next one a full window away, rather than
                    // re-connecting on every retry tick for as long as the node stays down.
                    consecutive_resolve_failures = 0;
                    tracing::warn!(
                        consecutive_failures = MAX_CONSECUTIVE_RESOLVE_FAILURES,
                        error = %format!("{err:#}"),
                        "🔌 Outbox resolution has failed for a full window — rebuilding the Creditcoin L1 EVM provider in place"
                    );
                    match connect_l1_provider(rpc.as_str()).await {
                        Ok(fresh) => {
                            provider = fresh;
                            tracing::info!(
                                "🔌 Creditcoin L1 EVM provider rebuilt; resuming Outbox resolution"
                            );
                        }
                        Err(e) => {
                            // Still unreachable. Keep retrying on the existing provider — the node
                            // being down is not a reason to take the whole attestor with it.
                            tracing::warn!(
                                error = %format!("{e:#}"),
                                "🔌 provider rebuild failed; continuing to retry"
                            );
                        }
                    }
                }
                // `{:#}` (alternate Display), not `%err`: these errors are built with
                // `anyhow::Context`, whose plain Display prints ONLY the outermost context. Logging
                // it that way reduced every failure to the bare phrase "…get_outbox_discovery_address()
                // reverted" and threw away the RPC/decode error underneath, which is why the message
                // below could only *guess* at the cause. The alternate form prints the whole chain.
                let err = format!("{err:#}");
                if resolve_attempts % RESOLVE_ESCALATE_EVERY_ATTEMPTS == 0 {
                    tracing::error!(
                        %err,
                        attempts = resolve_attempts,
                        elapsed_secs = resolve_attempts * OUTBOX_RESOLVE_RETRY_SECS,
                        "Outbox resolution still failing after prolonged retrying — the error chain above names the actual cause (RPC transport, revert, or chain-info selector mismatch); will keep retrying"
                    );
                } else {
                    tracing::warn!(%err, retry_secs = OUTBOX_RESOLVE_RETRY_SECS, "Outbox resolution failed — will retry");
                }
            }
        }
        tokio::select! {
            () = shared.token.cancelled() => {
                if let Some(watcher) = &set_watcher {
                    watcher.abort();
                }
                if let Some(proposer) = &set_update_proposer {
                    proposer.abort();
                }
                return Ok(());
            }
            joined = wait_for_optional_child(&mut set_watcher) => {
                if let Some(proposer) = &set_update_proposer {
                    proposer.abort();
                }
                // On shutdown these children exit cleanly with `Ok(())` and race the cancel branch,
                // so this arm winning must not turn an intentional stop into a task failure. The
                // post-activation loop already guards the same way.
                if shared.token.is_cancelled() {
                    return Ok(());
                }
                return Err(Error::WriteAbility(child_exit_error("attestor-set watcher", joined)));
            }
            joined = wait_for_optional_child(&mut set_update_proposer) => {
                if let Some(watcher) = &set_watcher {
                    watcher.abort();
                }
                // See the watcher arm: a clean child exit during shutdown must not be reported as a
                // failure.
                if shared.token.is_cancelled() {
                    return Ok(());
                }
                return Err(Error::WriteAbility(child_exit_error("attestor-set-update proposer", joined)));
            }
            () = tokio::time::sleep(std::time::Duration::from_secs(OUTBOX_RESOLVE_RETRY_SECS)) => {}
        }
    };
    tracing::info!(
        chain_key = resolved.chain_key,
        "✅ write-ability route activated"
    );

    let signer = signing::MessageSigner::from_seed(&seed).map_err(Error::WriteAbility)?;
    let our_address = signer.address();
    tracing::info!(
        evm_address = %our_address,
        "🔑 message-vote signer ready — register this address in the EOAValidator attestor set"
    );

    // Listener runs as a child task feeding us finalized messages; we sign, count, and publish.
    let (tx, mut rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let listener_token = shared.token.clone();
    let confirmation_depth = cfg.block_confirmation_depth;
    // Fall back to the pre-resolution head (not "now") so the resolve-wait window is covered.
    let scan_from = cfg.start_block.or(Some(head_before_resolve));
    // One durable cursor covers all Outboxes for this chain: a chunk advances only after every
    // candidate message has been authenticated at its historical source block and drained.
    let cursor_store = cursor::CursorStore::for_all_outboxes(
        &cfg.state_dir,
        cfg.write_ability_chain_key,
        cfg.start_block,
    );
    tracing::info!(
        path = %cursor_store.path().display(),
        "🗂️ persisting Outbox scan cursor across restarts"
    );
    let listener_tx = tx.clone();
    // One shared Creditcoin L1 handle for the listener and the reobservation worker. Alloy clones
    // share a pubsub backend, so a dead connection affects both. The listener detects a stall
    // (see `listener::MAX_CONSECUTIVE_POLL_FAILURES`),
    // rebuilds through the hook below and publishes the fresh provider here, and reobservation
    // clones the latest value before each use. Sibling of the resolver's in-place rebuild above;
    // nobody exits the task any more. The hook is an inline closure on purpose: its future yields
    // the very same opaque provider type the channel carries, where a named helper would mint a
    // second `impl Provider` that the listener's `P` could not unify with.
    let (l1_provider_tx, l1_provider_rx) = watch::channel(provider.clone());
    let listener_provider = l1_provider_tx.clone();
    let listener_rpc = rpc.clone();
    let listener_reconnect = move || {
        let rpc = listener_rpc.clone();
        async move { connect_l1_provider(rpc.as_str()).await }
    };
    let listener = tokio::spawn(async move {
        listener::watch(
            listener_provider,
            listener_reconnect,
            resolved,
            confirmation_depth,
            scan_from,
            cursor_store,
            listener_tx,
            listener_token,
        )
        .await
    });

    // One live routing view feeds both signing paths. Default-Outbox and Discovery-address
    // changes do not replace listeners: source-block authorization is resolved for each message.
    let (resolved_tx, mut resolved_rx) = watch::channel(Some(resolved));
    let mut outbox_monitor = tokio::spawn(run_outbox_monitor(
        cfg.clone(),
        cc3.clone(),
        state.clone(),
        shared.metrics.clone(),
        resolved,
        resolved_tx,
        shared.token.clone(),
    ));
    // `listener` becomes `None` while routing is paused; `active_outbox` retains log context.
    let mut listener = Some(listener);
    let mut active_outbox = Some(resolved);

    // Reobservation runs in its OWN task, NOT inline in this loop (audit P1-4): its tip + eth_getLogs
    // RPCs (deadline-bounded, serial) must not be able to stall vote production / message ingestion
    // if the shared provider black-holes. The bounded `reobs_rx` channel already drops excess, so a
    // flood is bounded to serial, deadline-capped work here.
    let mut reobs_worker = {
        let provider = l1_provider_rx.clone();
        let state = state.clone();
        let shared = shared.clone();
        let signer = signer.clone();
        let resolved_rx = resolved_rx.clone();
        tokio::spawn(run_reobservation_worker(
            provider,
            resolved_rx,
            state,
            shared,
            signer,
            our_address,
            confirmation_depth,
            reobs_rx,
        ))
    };

    let chain_key = shared.chain_key;
    loop {
        // Not `biased`: a biased select would always poll the listener channel before reobservation
        // requests, starving `reobs_rx` whenever indexed messages arrive continuously (catch-up or a
        // high publish rate) — exactly when relayer liveness recovery matters most. Random selection
        // keeps the two data channels fair; the cancellation token, once fired, stays ready and so is
        // still picked promptly (graceful shutdown tolerates finishing one in-flight item first).
        tokio::select! {
            () = shared.token.cancelled() => break,
            joined = async { listener.as_mut().expect("branch guarded by listener.is_some()").await }, if listener.is_some() => {
                // A clean `Ok(())` here during shutdown is the listener obeying the cancel token,
                // not an early exit; the same applies to every sibling arm below.
                if shared.token.is_cancelled() {
                    break;
                }
                let err = match joined {
                    Ok(Ok(())) => anyhow!("outbox listener exited without error or shutdown"),
                    Ok(Err(err)) => err.context("outbox listener died"),
                    Err(join_err) => anyhow!("outbox listener panicked: {join_err}"),
                };
                outbox_monitor.abort();
                reobs_worker.abort();
                if let Some(w) = &set_watcher {
                    w.abort();
                }
                if let Some(p) = &set_update_proposer {
                    p.abort();
                }
                return Err(Error::WriteAbility(err));
            }
            joined = wait_for_optional_child(&mut set_watcher) => {
                if shared.token.is_cancelled() {
                    break;
                }
                if let Some(l) = &listener {
                    l.abort();
                }
                outbox_monitor.abort();
                reobs_worker.abort();
                if let Some(p) = &set_update_proposer {
                    p.abort();
                }
                return Err(Error::WriteAbility(child_exit_error("attestor-set watcher", joined)));
            }
            joined = wait_for_optional_child(&mut set_update_proposer) => {
                if shared.token.is_cancelled() {
                    break;
                }
                if let Some(l) = &listener {
                    l.abort();
                }
                outbox_monitor.abort();
                reobs_worker.abort();
                if let Some(w) = &set_watcher {
                    w.abort();
                }
                return Err(Error::WriteAbility(child_exit_error("attestor-set-update proposer", joined)));
            }
            joined = &mut reobs_worker => {
                if shared.token.is_cancelled() {
                    break;
                }
                if let Some(l) = &listener {
                    l.abort();
                }
                outbox_monitor.abort();
                if let Some(w) = &set_watcher {
                    w.abort();
                }
                if let Some(p) = &set_update_proposer {
                    p.abort();
                }
                return Err(Error::WriteAbility(child_exit_error("reobservation worker", joined)));
            }
            joined = &mut outbox_monitor => {
                if shared.token.is_cancelled() {
                    break;
                }
                if let Some(l) = &listener {
                    l.abort();
                }
                reobs_worker.abort();
                if let Some(w) = &set_watcher {
                    w.abort();
                }
                if let Some(p) = &set_update_proposer {
                    p.abort();
                }
                return Err(Error::WriteAbility(child_exit_error("Outbox rotation monitor", joined)));
            }
            changed = resolved_rx.changed() => {
                if changed.is_err() {
                    // The monitor drops the sender when it returns on cancel, so this is the same
                    // clean-exit-during-shutdown race as the sibling arms above.
                    if shared.token.is_cancelled() {
                        break;
                    }
                    if let Some(l) = &listener {
                        l.abort();
                    }
                    reobs_worker.abort();
                    if let Some(w) = &set_watcher {
                        w.abort();
                    }
                    if let Some(p) = &set_update_proposer {
                        p.abort();
                    }
                    return Err(Error::WriteAbility(anyhow!(
                        "Outbox rotation monitor channel closed unexpectedly"
                    )));
                }
                let next = *resolved_rx.borrow_and_update();

                // Stop the old scanner before starting the new one: both cursor stores live in the
                // same per-chain state directory, so letting the writers overlap during the swap
                // would race. Awaiting the aborted task closes that window.
                if let Some(old_listener) = listener.take() {
                    old_listener.abort();
                    let _ = old_listener.await;
                }
                let old_outbox = active_outbox;
                active_outbox = next;

                if let Some(resolved) = next {
                    tracing::warn!(
                        ?old_outbox,
                        new_route = ?resolved,
                        "🔄 registry rotation detected — switching Outbox listener"
                    );
                    // Hand the replacement listener the shared handle, not a boot-time clone: if
                    // the previous listener rebuilt the connection, this one starts on the live one.
                    let listener_provider = l1_provider_tx.clone();
                    let listener_token = shared.token.clone();
                    let cursor_store = cursor::CursorStore::for_all_outboxes(&cfg.state_dir, cfg.write_ability_chain_key, cfg.start_block);
                    let listener_tx = tx.clone();
                    // Resume the same all-Outbox cursor after a signing-domain change. Any
                    // newly registered Outbox is discovered by the historical block scan itself.
                    let swap_start = scan_from;
                    let listener_rpc = rpc.clone();
                    let listener_reconnect = move || {
                        let rpc = listener_rpc.clone();
                        async move { connect_l1_provider(rpc.as_str()).await }
                    };
                    listener = Some(tokio::spawn(async move {
                        listener::watch(
                            listener_provider,
                            listener_reconnect,
                            resolved,
                            confirmation_depth,
                            swap_start,
                            cursor_store,
                            listener_tx,
                            listener_token,
                        )
                        .await
                    }));
                } else {
                    tracing::warn!(
                        ?old_outbox,
                        "⏸️ write-ability route disabled — signing paused"
                    );
                }
            }
            maybe = rx.recv() => {
                let Some(indexed) = maybe else {
                    // Unreachable while this task holds `tx` for rotation respawns, but kept
                    // defensively: a closed channel during shutdown is expected, not a fault.
                    if shared.token.is_cancelled() {
                        break;
                    }
                    // Otherwise a listener really did exit early. Harvest its result and surface
                    // the underlying error to the supervisor — otherwise it would only see a generic
                    // early-Ok exit and the failure reason would be lost. Abort every sibling task
                    // before surfacing the error — dropping a `JoinHandle` here would only detach
                    // it, leaving it running until the shared cancel token eventually fires.
                    let err = match listener.take() {
                        Some(handle) => match handle.await {
                            Ok(Ok(())) => anyhow!("outbox listener exited without error or shutdown"),
                            Ok(Err(err)) => err.context("outbox listener died"),
                            Err(join_err) => anyhow!("outbox listener panicked: {join_err}"),
                        },
                        None => anyhow!("Outbox message channel closed while no listener was running"),
                    };
                    outbox_monitor.abort();
                    reobs_worker.abort();
                    if let Some(w) = &set_watcher {
                        w.abort();
                    }
                    if let Some(p) = &set_update_proposer {
                        p.abort();
                    }
                    return Err(Error::WriteAbility(err));
                };
                // Default changes and scheduled removals do not invalidate historical messages.
                // Only a changed signing domain may make an already-authenticated buffer stale.
                let current = *resolved_rx.borrow();
                match current {
                    Some(active) if active.destination_chain_key == indexed.destination_chain_key => {
                        produce_vote(&state, &shared.metrics, &signer, our_address, chain_key, indexed);
                    }
                    Some(active) => {
                        tracing::warn!(
                            message_id = %indexed.message_id,
                            observed_on = %indexed.outbox,
                            active_route = ?active,
                            "dropping a message buffered under a superseded signing domain"
                        );
                    }
                    None => {
                        tracing::warn!(
                            message_id = %indexed.message_id,
                            observed_on = %indexed.outbox,
                            "dropping a buffered message while no Outbox is active"
                        );
                    }
                }
            }
        }
    }

    if let Some(listener) = listener {
        listener.abort();
    }
    outbox_monitor.abort();
    reobs_worker.abort();
    if let Some(w) = set_watcher {
        w.abort();
    }
    if let Some(p) = set_update_proposer {
        p.abort();
    }
    Ok(())
}

/// Await an optional child without making the select branch ready when that child is disabled.
async fn wait_for_optional_child(
    child: &mut Option<tokio::task::JoinHandle<()>>,
) -> Result<(), tokio::task::JoinError> {
    match child {
        Some(child) => child.await,
        None => std::future::pending().await,
    }
}

fn child_exit_error(
    name: &'static str,
    joined: Result<(), tokio::task::JoinError>,
) -> anyhow::Error {
    match joined {
        Ok(()) => anyhow!("{name} exited before shutdown was requested"),
        Err(err) => anyhow!("{name} panicked or was cancelled unexpectedly: {err}"),
    }
}

/// Build a new route from the immutable source identity established during activation. A rekey
/// must not wait for another EVM RPC after publishing the authorization: otherwise a failed chain
/// ID read leaves the old listener advancing its cursor while all its messages are rejected.
fn refresh_governance_route(
    state: &MessageVoteState,
    metrics: &metrics::Metrics,
    established: resolver::ResolvedRoute,
    result: anyhow::Result<Option<B256>>,
) -> anyhow::Result<Option<resolver::ResolvedRoute>> {
    let key = apply_governance_read(state, metrics, result)?;
    Ok(key.map(|destination_chain_key| resolver::ResolvedRoute {
        destination_chain_key,
        ..established
    }))
}

/// Refresh finalized governance and routing after activation. An explicitly disabled governance
/// entry pauses both signing paths; a changed destination key replaces the routing snapshot.
/// Failed refreshes preserve the last successful configuration and report degraded governance.
async fn run_outbox_monitor(
    cfg: Config,
    cc3: Arc<cc_client::Client>,
    state: Arc<MessageVoteState>,
    metrics: metrics::Metrics,
    current: resolver::ResolvedRoute,
    resolved_tx: watch::Sender<Option<resolver::ResolvedRoute>>,
    token: tokio_util::sync::CancellationToken,
) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(OUTBOX_RESOLVE_RETRY_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut active = Some(current);
    loop {
        tokio::select! {
            () = token.cancelled() => return,
            _ = tick.tick() => {
                let governance = resolve_destination_chain_key(&cfg, &cc3).await;
                // No await or fallible EVM lookup may separate the authorization update from
                // publishing its matching route. Source identity was validated at activation.
                let attempt = refresh_governance_route(&state, &metrics, current, governance);
                match rotation_action(&attempt, active) {
                    RotationAction::Swap(next) => {
                        tracing::info!(
                            old = ?active,
                            new = ?next,
                            "🧭 replacement Outbox resolved"
                        );
                        active = Some(next);
                        if resolved_tx.send(Some(next)).is_err() {
                            return;
                        }
                    }
                    RotationAction::Pause => {
                        tracing::warn!(
                            old = ?active,
                            "write-ability route is no longer enabled — pausing signing"
                        );
                        active = None;
                        if resolved_tx.send(None).is_err() {
                            return;
                        }
                    }
                    RotationAction::Nothing => {}
                }
            }
        }
    }
}

/// What one rotation-monitor tick should do. Pure so it is testable without a live provider.
#[derive(Debug, PartialEq)]
enum RotationAction {
    /// The signing domain changed — replace the listener's routing snapshot.
    Swap(resolver::ResolvedRoute),
    /// Routing is disabled — publish `None` so signing stops.
    Pause,
    Nothing,
}

/// A transient RPC failure does not establish a different route. Historical membership checks
/// still fail closed in the listener and reobservation paths while the provider is unavailable.
fn rotation_action(
    attempt: &anyhow::Result<Option<resolver::ResolvedRoute>>,
    active: Option<resolver::ResolvedRoute>,
) -> RotationAction {
    match attempt {
        Ok(Some(next)) if active != Some(*next) => RotationAction::Swap(*next),
        Ok(Some(_)) => RotationAction::Nothing,
        Ok(None) if active.is_some() => RotationAction::Pause,
        Ok(None) | Err(_) => RotationAction::Nothing,
    }
}

/// Reobservation responder, run as its own task (audit P1-4). Consumes verified-on-request pull
/// requests off `reobs_rx`, re-fetches + re-signs one at a time under a wall-clock deadline, so a
/// slow/black-holed RPC can never stall the main write-ability loop (vote production + ingestion).
/// The bounded `reobs_rx` channel drops excess at the p2p ingest, so a flood is naturally bounded to
/// serial, deadline-capped work here.
#[allow(clippy::too_many_arguments)]
async fn run_reobservation_worker<P: alloy::providers::Provider + Clone>(
    provider_rx: watch::Receiver<P>,
    mut resolved_rx: watch::Receiver<Option<resolver::ResolvedRoute>>,
    state: Arc<MessageVoteState>,
    shared: Arc<Shared>,
    signer: signing::MessageSigner,
    our_address: Address,
    confirmation_depth: u64,
    mut reobs_rx: mpsc::Receiver<ReobservationRequest>,
) {
    // Per-`message_id` cooldown so a spammed/forged request can't make us re-scan the chain in a loop.
    let mut limiter = reobservation::ReobsRateLimiter::new(reobservation::REOBS_MIN_INTERVAL);
    let chain_key = shared.chain_key;
    loop {
        tokio::select! {
            () = shared.token.cancelled() => return,
            maybe = reobs_rx.recv() => {
                let Some(request) = maybe else {
                    tracing::debug!("reobservation channel closed — worker exiting");
                    return;
                };
                // `borrow_and_update`, not `borrow`: this snapshot must also mark the value seen.
                // With a plain `borrow` a rotation that landed before this request was picked up
                // leaves the receiver still flagged as changed, so the `changed()` branch below
                // fires immediately and abandons work whose snapshot was already current — dropping
                // the first recovery attempt after every rotation.
                let Some(resolved) = *resolved_rx.borrow_and_update() else {
                    tracing::warn!(message_id = ?request.message_id, "dropping reobservation request while no Outbox is active");
                    continue;
                };
                // Latest shared connection, cloned as its own statement so the watch read guard is
                // released before the deadline-capped RPC work below.
                let provider = provider_rx.borrow().clone();
                let handle = handle_reobservation(
                    &provider, &resolved, &state, &shared.metrics, &signer, our_address, chain_key,
                    confirmation_depth, &mut limiter, request,
                );
                // Routing is snapshotted above. Abandon in-flight work if its signing domain
                // changes; default changes and removals do not change this route or cancel recovery.
                tokio::select! {
                    () = shared.token.cancelled() => return,
                    changed = resolved_rx.changed() => {
                        if changed.is_err() {
                            tracing::debug!("Outbox rotation channel closed — reobservation worker exiting");
                            return;
                        }
                        tracing::warn!(
                            route = ?resolved,
                            "abandoning an in-flight reobservation — the Outbox rotated mid-request"
                        );
                    }
                    outcome = tokio::time::timeout(reobservation::REOBS_RPC_TIMEOUT, handle) => {
                        if outcome.is_err() {
                            tracing::warn!(
                                "reobservation re-fetch exceeded {:?} — RPC unresponsive; dropping this request",
                                reobservation::REOBS_RPC_TIMEOUT
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Sign our vote for a freshly indexed message, count it locally (chain-seen + our own signature),
/// and hand it to the p2p task to gossip.
fn produce_vote(
    state: &MessageVoteState,
    metrics: &metrics::Metrics,
    signer: &signing::MessageSigner,
    our_address: Address,
    chain_key: u64,
    indexed: listener::IndexedMessage,
) {
    // Keep this guard until the signature is queued. Both normal indexing and reobservation pass
    // through here; a governance refresh cannot race the final authorization check.
    let governance = state.destination_chain_key.read();
    if *governance != Some(indexed.destination_chain_key) {
        tracing::debug!(message_id = %indexed.message_id, "message signing paused or destination key changed");
        return;
    }
    let signature = match signer.sign(&indexed.message_hash) {
        Ok(sig) => sig,
        Err(err) => {
            tracing::error!(%err, message_id = %indexed.message_id, "failed to sign message vote");
            return;
        }
    };
    // Liveness signal for the local signing pipeline: a flat produced-rate while the chain still has
    // Outbox activity means we stopped signing even though incoming peer votes may keep arriving (S4).
    metrics.note_message_vote_produced();

    // Chain-seen (we observed it on-chain) + count our own vote — but only tally our signature
    // toward local quorum when our address is actually in the authorized set. Peers reject votes
    // from non-attestors, so counting our own unconditionally would let a misconfigured node (a
    // signer key that isn't in the on-chain EOAValidator set) log a false "threshold reached" while
    // the relayer still lacks enough valid votes. We still gossip below regardless: if the on-chain
    // set was just updated to include us but our local view hasn't refreshed yet (30s poll), the
    // relayer counts our vote even though we don't count it locally for another tick.
    let authorized = state.active_set.read().contains(&our_address);
    {
        let now = Instant::now();
        let mut agg = state.aggregator.lock();
        agg.note_indexed(indexed.message_hash.0, now);
        if authorized {
            if let aggregator::VoteOutcome::Accepted {
                reached_threshold: true,
            } = agg.add_vote(indexed.message_hash.0, our_address, now)
            {
                ingest::note_threshold(chain_key, &indexed.message_hash);
            }
        }
    }
    if !authorized {
        tracing::warn!(
            %our_address,
            "⚠️ our signer is not in the active attestor set — gossiping our vote but not counting it locally"
        );
    }

    let vote = MessageVote {
        chain_key,
        message_id: indexed.message_id.0,
        message_hash: indexed.message_hash.0,
        signer: our_address.into_array(),
        signature,
    };

    // `try_send` (not `send().await`) so a wedged/backed-up p2p task can never apply backpressure
    // into this loop. A dropped vote is recoverable: the gossipsub mesh re-gossips from peers that
    // did receive it, and the relayer's reobservation request re-drives `produce_vote` when a
    // message sits below quorum — by which point the channel has typically drained. Mirrors the
    // block-attestation broadcast in `production.rs`.
    match state.publish_tx.try_send(vote) {
        Ok(()) => tracing::info!(
            message_id = %indexed.message_id,
            message_hash = %indexed.message_hash,
            "✉️ queued message vote for gossip"
        ),
        Err(mpsc::error::TrySendError::Full(_)) => tracing::warn!(
            message_id = %indexed.message_id,
            "📭 message-vote channel full — dropping broadcast (recovered via gossip/reobservation)"
        ),
        Err(mpsc::error::TrySendError::Closed(_)) => tracing::warn!(
            message_id = %indexed.message_id,
            "📭 message-vote channel closed — p2p task exited"
        ),
    }
}

/// Honor a reobservation request (liveness recovery): rate-limit per `message_id`, independently
/// re-verify the message against our own RPC, skip if we've already seen local quorum for it, then
/// re-sign + re-gossip exactly as if we'd just indexed it. Errors and unverifiable requests are
/// logged and dropped — never fatal.
#[allow(clippy::too_many_arguments)]
async fn handle_reobservation<P: alloy::providers::Provider>(
    provider: &P,
    resolved: &resolver::ResolvedRoute,
    state: &MessageVoteState,
    metrics: &metrics::Metrics,
    signer: &signing::MessageSigner,
    our_address: Address,
    chain_key: u64,
    confirmation_depth: u64,
    limiter: &mut reobservation::ReobsRateLimiter,
    request: ReobservationRequest,
) {
    let message_id = alloy::primitives::B256::from(request.message_id);
    if request.chain_key != chain_key {
        return; // not ours (topic is per-chain, but be defensive)
    }
    let now = Instant::now();
    if !limiter.allow(message_id, now) {
        tracing::debug!(%message_id, "⏳ reobservation request within cooldown — ignoring");
        return;
    }

    // Record the cooldown *after* verifying, and only apply the full cooldown to a verified
    // request. An unauthenticated forged/garbage request gets the short failure cooldown so it can't
    // burn this message's 30s window and starve the genuine relayer request (S2).
    let indexed = match reobservation::reobserve(provider, resolved, confirmation_depth, &request)
        .await
    {
        Ok(Some(indexed)) => {
            limiter.record(message_id, now, true);
            indexed
        }
        Ok(None) => {
            limiter.record(message_id, now, false);
            tracing::warn!(
                %message_id,
                block = request.block_height,
                "🔎 reobservation request did not match a verifiable MessagePublished — ignoring"
            );
            return;
        }
        Err(err) => {
            limiter.record(message_id, now, false);
            tracing::warn!(%message_id, %err, "reobservation re-fetch failed — ignoring");
            return;
        }
    };

    // Re-sign unconditionally (the per-`message_id` cooldown above is the bound). We must NOT skip
    // just because our *own* aggregator already saw local quorum: the requester is the relayer, and
    // the whole reason it asked is that it is missing votes the attestor mesh may have settled among
    // itself. Re-gossiping is idempotent at the relayer (it dedups), so the worst case is harmless.
    tracing::info!(
        %message_id,
        message_hash = %indexed.message_hash,
        "♻️ re-signing reobserved message"
    );
    produce_vote(state, metrics, signer, our_address, chain_key, indexed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    fn route(addr: Address) -> resolver::ResolvedRoute {
        resolver::ResolvedRoute {
            chain_key: 7,
            destination_chain_key: addr.into_word(),
            creditcoin_chain_id: 42,
        }
    }

    // The rebuild path added when the resolver stopped killing the process: a dead endpoint must
    // come back as `Err` so the loop can log it and keep going. If `on_builtin` ever became lazy
    // for this URL scheme it would hand back a healthy-looking provider instead, and the rebuild
    // would silently swap a dead connection for another dead connection.
    #[tokio::test]
    async fn connect_l1_provider_reports_a_dead_endpoint_as_an_error() {
        // Port 1 is privileged and never bound by the test host, so this refuses immediately
        // rather than spending the full RPC_ATTEMPT_TIMEOUT.
        let err = connect_l1_provider("ws://127.0.0.1:1")
            .await
            .err()
            .expect("a websocket endpoint that refuses the connection must not yield a provider");
        assert!(
            format!("{err:#}").contains("connect Creditcoin L1 EVM RPC"),
            "the failure should be labelled as an L1 connect failure, got: {err:#}"
        );
    }

    // Sizing guard for the rebuild window. The CI outage-recovery scenarios bounce the node for
    // `RANDOM % 180` seconds plus restart time; the window has to sit above that so an orchestrated
    // restart rides out on plain retries and only a genuinely wedged provider triggers a rebuild.
    #[test]
    fn resolve_failure_window_outlasts_an_orchestrated_node_restart() {
        let window_secs = MAX_CONSECUTIVE_RESOLVE_FAILURES * OUTBOX_RESOLVE_RETRY_SECS;
        assert_eq!(window_secs, 300, "the rebuild window should be ~5 minutes");
        assert!(
            window_secs > 180,
            "window ({window_secs}s) must exceed the CI outage sleep of up to 180s"
        );
    }

    const KEY_A: Address = address!("00000000000000000000000000000000000000aa");
    const KEY_B: Address = address!("00000000000000000000000000000000000000bb");

    #[test]
    fn disabled_route_pauses_only_when_something_was_active() {
        let none: anyhow::Result<Option<resolver::ResolvedRoute>> = Ok(None);
        assert_eq!(
            rotation_action(&none, Some(route(KEY_A))),
            RotationAction::Pause
        );
        // Nothing active (already paused, or never resolved) — nothing to pause.
        assert_eq!(rotation_action(&none, None), RotationAction::Nothing);
    }

    // A registry read is atomic: a failed attempt carries no information about whether the
    // registration actually changed, so it must never pause an already-active listener — only a
    // clean `Ok(None)` does. Retried on the next tick, same as any other transient RPC failure.
    #[test]
    fn a_failed_attempt_never_pauses() {
        let err: anyhow::Result<Option<resolver::ResolvedRoute>> = Err(anyhow!("rpc blip"));
        assert_eq!(
            rotation_action(&err, Some(route(KEY_A))),
            RotationAction::Nothing
        );
        assert_eq!(rotation_action(&err, None), RotationAction::Nothing);
    }

    #[test]
    fn changed_domain_swaps_and_same_route_does_nothing() {
        let next = route(KEY_B);
        let attempt: anyhow::Result<Option<resolver::ResolvedRoute>> = Ok(Some(next));
        assert_eq!(
            rotation_action(&attempt, Some(route(KEY_A))),
            RotationAction::Swap(next),
        );
        // Resuming from a pause is a swap too (nothing was active).
        assert_eq!(rotation_action(&attempt, None), RotationAction::Swap(next));
        // Same address re-resolved: nothing to do.
        let same: anyhow::Result<Option<resolver::ResolvedRoute>> = Ok(Some(route(KEY_A)));
        assert_eq!(
            rotation_action(&same, Some(route(KEY_A))),
            RotationAction::Nothing,
        );
    }

    fn test_metrics() -> metrics::Metrics {
        metrics::Metrics::new(
            metrics::ConfigBuilder::new()
                .with_name("governance-test")
                .with_address(cc_client::AccountId32::from([0; 32]))
                .with_peer_id(libp2p::PeerId::random())
                .with_chain_key(7u64)
                .with_start_height(0u64)
                .with_start_attestation(None)
                .with_genesis(0u64)
                .with_attestation_latest_eth(0u64)
                .with_attestation_interval(std::num::NonZeroU64::new(1).unwrap())
                .build(),
        )
    }

    fn test_state(
        signer: &signing::MessageSigner,
    ) -> (MessageVoteState, mpsc::Receiver<MessageVote>) {
        let (publish_tx, rx) = mpsc::channel(8);
        (
            MessageVoteState {
                aggregator: Mutex::new(aggregator::VoteAggregator::new(
                    1,
                    100,
                    Duration::from_secs(60),
                )),
                active_set: RwLock::new(HashSet::from([signer.address()])),
                publish_tx,
                set_update_publish_tx: mpsc::channel(8).0,
                reobs_tx: mpsc::channel(8).0,
                destination_chain_key: RwLock::new(None),
            },
            rx,
        )
    }

    fn message(key: B256) -> listener::IndexedMessage {
        let message_id = B256::repeat_byte(1);
        let emitter = KEY_B;
        let payload = vec![42];
        listener::IndexedMessage {
            message_id,
            emitter,
            outbox: KEY_A,
            destination_chain_key: key,
            message_hash: write_ability::hash::message_hash(
                message_id, emitter, KEY_A, key, 42, &payload,
            ),
            payload,
        }
    }

    #[test]
    fn governance_disables_and_reenables_the_actual_signing_path() {
        let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
        let (state, mut votes) = test_state(&signer);
        let metrics = test_metrics();
        let key = chain_key_to_bytes32(7);
        let sign = || produce_vote(&state, &metrics, &signer, signer.address(), 7, message(key));
        // Starts paused (including boot while disabled or while governance RPC is unavailable).
        sign();
        assert!(votes.try_recv().is_err());
        apply_governance(&state, Some(key));
        sign();
        assert!(votes.try_recv().is_ok());
        assert_eq!(state.aggregator.lock().tracked(), 1);
        apply_governance(&state, None);
        assert_eq!(state.aggregator.lock().tracked(), 0);
        sign();
        assert!(votes.try_recv().is_err());
        apply_governance(&state, Some(key));
        sign();
        assert!(
            votes.try_recv().is_ok(),
            "reenablement must not require a process restart"
        );
    }

    #[test]
    fn destination_change_rejects_buffered_and_inflight_old_key_messages() {
        let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
        let (state, mut votes) = test_state(&signer);
        let metrics = test_metrics();
        let old_key = chain_key_to_bytes32(7);
        let new_key = chain_key_to_bytes32(8);
        apply_governance(&state, Some(old_key));
        let in_flight = message(old_key);
        apply_governance(&state, Some(new_key));
        produce_vote(&state, &metrics, &signer, signer.address(), 7, in_flight);
        assert!(votes.try_recv().is_err());
        produce_vote(
            &state,
            &metrics,
            &signer,
            signer.address(),
            7,
            message(new_key),
        );
        assert!(votes.try_recv().is_ok());
        let mut old = route(KEY_A);
        old.destination_chain_key = old_key;
        let mut next = old;
        next.destination_chain_key = new_key;
        assert_eq!(
            rotation_action(&Ok(Some(next)), Some(old)),
            RotationAction::Swap(next)
        );
    }

    #[test]
    fn governance_clear_preserves_the_validator_threshold() {
        let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
        let (state, _) = test_state(&signer);
        state.aggregator.lock().set_threshold(2, Instant::now());
        apply_governance(&state, Some(chain_key_to_bytes32(7)));
        let mut aggregator = state.aggregator.lock();
        let hash = [1; 32];
        aggregator.note_indexed(hash, Instant::now());
        assert_eq!(
            aggregator.add_vote(hash, signer.address(), Instant::now()),
            aggregator::VoteOutcome::Accepted {
                reached_threshold: false
            }
        );
    }

    async fn failed_governance_read(timeout: bool) -> anyhow::Result<Option<B256>> {
        use supported_chains_primitives::WriteAbilityConfig;
        if timeout {
            let stalled = std::future::pending::<Result<Option<WriteAbilityConfig>, &str>>();
            read_governance(7, stalled, Duration::from_millis(1)).await
        } else {
            let failure =
                std::future::ready(Err::<Option<WriteAbilityConfig>, _>("RPC unavailable"));
            read_governance(7, failure, Duration::from_secs(1)).await
        }
    }

    fn assert_governance_degraded(metrics: &metrics::Metrics, expected: bool) {
        assert!(metrics.encode().lines().any(|line| {
            line == format!("write_ability_governance_degraded {}", u64::from(expected))
        }));
    }

    #[tokio::test]
    async fn governance_startup_failure_keeps_signing_paused_and_reports_degraded() {
        for timeout in [false, true] {
            let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
            let (state, mut votes) = test_state(&signer);
            let metrics = test_metrics();
            let result =
                apply_governance_read(&state, &metrics, failed_governance_read(timeout).await);
            assert!(result.is_err());
            assert_eq!(*state.destination_chain_key.read(), None);
            assert_governance_degraded(&metrics, true);
            produce_vote(
                &state,
                &metrics,
                &signer,
                signer.address(),
                7,
                message(chain_key_to_bytes32(7)),
            );
            assert!(votes.try_recv().is_err());
            assert_eq!(state.aggregator.lock().tracked(), 0);
        }
    }

    #[tokio::test]
    async fn governance_failures_preserve_signing_routing_and_inflight_quorum_until_disable() {
        for timeout in [false, true] {
            let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
            let (state, mut votes) = test_state(&signer);
            let metrics = test_metrics();
            let key = chain_key_to_bytes32(7);
            apply_governance_read(&state, &metrics, Ok(Some(key))).unwrap();
            state.aggregator.lock().set_threshold(2, Instant::now());
            let sign = || {
                produce_vote(&state, &metrics, &signer, signer.address(), 7, message(key));
            };
            sign();
            let first_vote = votes.try_recv().unwrap();
            assert_eq!(
                state
                    .aggregator
                    .lock()
                    .signer_count(&first_vote.message_hash),
                1
            );

            let result =
                apply_governance_read(&state, &metrics, failed_governance_read(timeout).await);
            assert!(result.is_err());
            assert_eq!(*state.destination_chain_key.read(), Some(key));
            // The same failed refresh reaches the monitor; it cannot publish a route pause and
            // abort the running listener or reobservation worker.
            let route_result = result.map(|_| None);
            assert_eq!(
                rotation_action(&route_result, Some(route(KEY_A))),
                RotationAction::Nothing
            );
            assert_governance_degraded(&metrics, true);
            sign();
            assert!(votes.try_recv().is_ok(), "signing continues while degraded");
            assert_eq!(state.aggregator.lock().tracked(), 1);
            assert_eq!(
                state
                    .aggregator
                    .lock()
                    .add_vote(first_vote.message_hash, KEY_B, Instant::now(),),
                aggregator::VoteOutcome::Accepted {
                    reached_threshold: true
                },
                "the pre-outage vote must still count toward quorum"
            );

            let disabled = apply_governance_read(&state, &metrics, Ok(None)).unwrap();
            assert_eq!(disabled, None);
            assert_eq!(
                rotation_action(&Ok(None), Some(route(KEY_A))),
                RotationAction::Pause
            );
            assert_governance_degraded(&metrics, false);
            assert_eq!(state.aggregator.lock().tracked(), 0);
            sign();
            assert!(votes.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn governance_recovery_rekeys_without_another_chain_id_rpc_and_clears_stale_votes() {
        let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
        let (state, mut votes) = test_state(&signer);
        let metrics = test_metrics();
        let old_key = chain_key_to_bytes32(7);
        let new_key = chain_key_to_bytes32(8);
        apply_governance_read(&state, &metrics, Ok(Some(old_key))).unwrap();
        produce_vote(
            &state,
            &metrics,
            &signer,
            signer.address(),
            7,
            message(old_key),
        );
        votes.try_recv().unwrap();
        assert!(
            apply_governance_read(&state, &metrics, failed_governance_read(false).await).is_err()
        );
        assert_governance_degraded(&metrics, true);
        assert_eq!(state.aggregator.lock().tracked(), 1);

        // The EVM provider is deliberately not involved: even if eth_chainId is now failing,
        // a successful governance read must immediately produce a matching replacement route.
        let mut established = route(KEY_A);
        established.destination_chain_key = old_key;
        let next = refresh_governance_route(&state, &metrics, established, Ok(Some(new_key)))
            .unwrap()
            .unwrap();
        assert_eq!(next.creditcoin_chain_id, established.creditcoin_chain_id);
        assert_eq!(next.chain_key, established.chain_key);
        assert_eq!(next.destination_chain_key, new_key);
        assert_eq!(
            rotation_action(&Ok(Some(next)), Some(established)),
            RotationAction::Swap(next),
            "a successful rekey cannot retain the old scanner's signing domain"
        );
        assert_governance_degraded(&metrics, false);
        assert_eq!(state.aggregator.lock().tracked(), 0);
        produce_vote(
            &state,
            &metrics,
            &signer,
            signer.address(),
            7,
            message(old_key),
        );
        assert!(
            votes.try_recv().is_err(),
            "stale buffered work must be rejected"
        );
        produce_vote(
            &state,
            &metrics,
            &signer,
            signer.address(),
            7,
            message(new_key),
        );
        assert!(votes.try_recv().is_ok());
    }

    #[tokio::test]
    async fn governance_failure_after_successful_disable_never_reenables() {
        for timeout in [false, true] {
            let signer = signing::MessageSigner::from_seed(&[7; 32]).unwrap();
            let (state, mut votes) = test_state(&signer);
            let metrics = test_metrics();
            let key = chain_key_to_bytes32(7);
            apply_governance_read(&state, &metrics, Ok(None)).unwrap();
            assert_governance_degraded(&metrics, false);
            assert!(
                apply_governance_read(&state, &metrics, failed_governance_read(timeout).await)
                    .is_err()
            );
            assert_governance_degraded(&metrics, true);
            assert_eq!(*state.destination_chain_key.read(), None);
            produce_vote(&state, &metrics, &signer, signer.address(), 7, message(key));
            assert!(votes.try_recv().is_err());

            apply_governance_read(&state, &metrics, Ok(Some(key))).unwrap();
            assert_governance_degraded(&metrics, false);
            produce_vote(&state, &metrics, &signer, signer.address(), 7, message(key));
            assert!(votes.try_recv().is_ok());
        }
    }

    #[tokio::test]
    async fn governance_read_errors_and_timeouts_never_enable_local_fallback() {
        use supported_chains_primitives::WriteAbilityConfig;
        let failure = std::future::ready(Err::<Option<WriteAbilityConfig>, _>("RPC unavailable"));
        assert!(read_governance(7, failure, Duration::from_secs(1))
            .await
            .is_err());
        let stalled = std::future::pending::<Result<Option<WriteAbilityConfig>, &str>>();
        assert!(read_governance(7, stalled, Duration::from_millis(1))
            .await
            .is_err());
        let missing = std::future::ready(Ok::<Option<WriteAbilityConfig>, &str>(None));
        assert_eq!(
            read_governance(7, missing, Duration::from_secs(1))
                .await
                .unwrap(),
            Some(chain_key_to_bytes32(7))
        );
        for enabled in [false, true] {
            let onchain = WriteAbilityConfig {
                write_ability_chain_key: [9; 32],
                message_attestation_enabled: enabled,
            };
            let read = std::future::ready(Ok::<_, &str>(Some(onchain)));
            assert_eq!(
                read_governance(7, read, Duration::from_secs(1))
                    .await
                    .unwrap(),
                enabled.then_some(B256::repeat_byte(9))
            );
        }
    }
}
