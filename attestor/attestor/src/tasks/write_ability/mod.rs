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
/// chain-info selector (`get_outbox_factory_address`) reverts and resolution can never succeed (S3).
/// Escalate the log to error-level at each multiple so it is alertable instead of buried in warns.
const RESOLVE_ESCALATE_EVERY_ATTEMPTS: u64 = (5 * 60) / OUTBOX_RESOLVE_RETRY_SECS;

/// A responsive node may legitimately report that no Outbox exists yet (`Ok(None)`) forever, but
/// repeated RPC errors against the same bare alloy provider mean the connection is no longer
/// usable. Surface the failure so the process supervisor rebuilds the provider on restart instead
/// of retrying the same dead socket indefinitely.
///
/// Sized to ~5 minutes (matching [`RESOLVE_ESCALATE_EVERY_ATTEMPTS`]): a *planned* node restart —
/// the CI outage-recovery scenarios bounce the CC3 node for a couple of minutes, and a devnet
/// rollout looks the same — must ride out on retries, not exit the process. Only an outage long
/// past any orchestrated restart indicates the provider itself is wedged.
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
    /// The `bytes32` write-ability chain key bound into every `messageHash` and passed to
    /// `IOutboxFactory.getOutbox`. Sourced from the on-chain `WriteAbilityConfigs` entry when one
    /// is registered for this `chain_key`; derived locally (right-padded `u64`) otherwise.
    pub destination_chain_key: B256,
}

/// Build the shared message-vote state and the matching publish channel receiver from config, or
/// `None` when message attestation is disabled / not yet supported. Runs during `lib.rs` startup
/// before tasks spawn; resolving an [`AttestorSet::OnChainValidator`] set performs one RPC read.
///
/// Enablement is gated twice: the local `enabled` flag is the *operator's* opt-in (and implies the
/// RPC endpoints are configured), while the on-chain `WriteAbilityConfigs` entry for `chain_key` is
/// *governance's* switch — when an entry exists with `message_attestation_enabled == false` the
/// task stays off regardless of local config. A missing entry (or a failed read) falls back to
/// local config so dev setups and chain outages don't disable a configured attestor. On-chain
/// changes are picked up on restart.
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
    let destination_chain_key = resolve_destination_chain_key(cfg, cc3).await?;
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
        destination_chain_key,
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
/// `bytes32` write-ability chain key. Returns `None` when governance has explicitly disabled
/// message attestation for the chain (an entry exists with `message_attestation_enabled == false`).
async fn resolve_destination_chain_key(cfg: &Config, cc3: &cc_client::Client) -> Option<B256> {
    let chain_key = cfg.write_ability_chain_key;
    let local = chain_key_to_bytes32(chain_key);
    let config =
        tokio::time::timeout(RPC_ATTEMPT_TIMEOUT, cc3.get_write_ability_config(chain_key)).await;
    match config {
        Ok(Ok(Some(on_chain))) => {
            if !on_chain.message_attestation_enabled {
                tracing::info!(
                    chain_key,
                    "📴 on-chain WriteAbilityConfig disables message attestation for this chain — disabling"
                );
                return None;
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
            Some(key)
        }
        Ok(Ok(None)) => {
            tracing::warn!(
                chain_key,
                "no on-chain WriteAbilityConfig registered for this chain — using the locally derived chain key"
            );
            Some(local)
        }
        Ok(Err(err)) => {
            // Availability over strictness: a transient read failure must not disable a locally
            // configured attestor. Explicit governance "off" is only honored via Ok(Some(..)).
            tracing::warn!(
                chain_key,
                %err,
                "failed to read on-chain WriteAbilityConfig — falling back to local config"
            );
            Some(local)
        }
        Err(_) => {
            tracing::warn!(
                chain_key,
                timeout_secs = RPC_ATTEMPT_TIMEOUT.as_secs(),
                "timed out reading on-chain WriteAbilityConfig — falling back to local config"
            );
            Some(local)
        }
    }
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
    let provider = tokio::time::timeout(
        RPC_ATTEMPT_TIMEOUT,
        ProviderBuilder::new().on_builtin(rpc.as_str()),
    )
    .await
    .map_err(|_| {
        Error::WriteAbility(anyhow!(
            "connect Creditcoin L1 EVM RPC timed out after {RPC_ATTEMPT_TIMEOUT:?}"
        ))
    })?
    .map_err(|e| Error::WriteAbility(anyhow!("connect Creditcoin L1 EVM RPC: {e}")))?;

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
    // an attestor can be started before the factory/Outbox is registered on-chain and will activate
    // write-ability automatically once they are, with no restart. While unresolved it just keeps
    // doing block attestation. The discovery cursor stays live after activation in
    // `run_outbox_monitor`, which detects factory and Outbox rotations with the same polling.
    let mut resolve_attempts: u64 = 0;
    let mut consecutive_resolve_failures: u64 = 0;
    // Discovery cursor: advances past confirmed blocks already scanned for `OutboxCreated` so each
    // retry only scans new blocks instead of re-scanning the whole chain history every interval.
    let mut outbox_cursor = resolver::OutboxDiscoveryCursor::default();
    let resolved = loop {
        // Progress-aware failure budget, mirroring the listener's `next_failure_count`. Outbox
        // discovery is a *chunked* log scan, so one attempt legitimately exceeds
        // `RPC_ATTEMPT_TIMEOUT` on a long chain while still advancing the cursor each chunk.
        // Counting those as failures would trip `MAX_CONSECUTIVE_RESOLVE_FAILURES` and restart —
        // and `outbox_cursor` is in-memory, so the restart discards every chunk already scanned,
        // making it a loop that never activates rather than a recovery.
        let scanned_before = outbox_cursor.scanned_to();
        let attempt = tokio::select! {
            attempt = tokio::time::timeout(
                RPC_ATTEMPT_TIMEOUT,
                resolver::resolve(
                    &provider,
                    &cfg,
                    state.destination_chain_key,
                    &mut outbox_cursor,
                ),
            ) => attempt.unwrap_or_else(|_| {
                Err(anyhow!(
                    "Outbox resolution RPC attempt timed out after {RPC_ATTEMPT_TIMEOUT:?}"
                ))
            }),
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
                        "⏳ Outbox still unresolved after prolonged retrying — if this chain is meant to serve write-ability, verify the on-chain WriteAbilityConfigs entry and the runtime/attestor deploy ordering (chain-info `get_outbox_factory_address` selector)"
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
                if outbox_cursor.scanned_to() > scanned_before {
                    // Advanced before failing: this is a wide scan in progress, not a dead endpoint.
                    consecutive_resolve_failures = 0;
                    tracing::info!(
                        scanned_to = outbox_cursor.scanned_to(),
                        error = %format!("{err:#}"),
                        "🐢 Outbox discovery advanced but did not finish this attempt; continuing (not counted as a failure)"
                    );
                } else {
                    consecutive_resolve_failures += 1;
                }
                if consecutive_resolve_failures >= MAX_CONSECUTIVE_RESOLVE_FAILURES {
                    return Err(Error::WriteAbility(err.context(format!(
                        "Outbox resolution failed {consecutive_resolve_failures} consecutive times; restarting to rebuild the RPC provider"
                    ))));
                }
                // `{:#}` (alternate Display), not `%err`: these errors are built with
                // `anyhow::Context`, whose plain Display prints ONLY the outermost context. Logging
                // it that way reduced every failure to the bare phrase "…get_outbox_factory_address()
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
    tracing::info!(outbox = %resolved.address, "✅ write-ability activated — Outbox resolved");

    let signer = signing::MessageSigner::from_seed(&seed).map_err(Error::WriteAbility)?;
    let our_address = signer.address();
    tracing::info!(
        evm_address = %our_address,
        "🔑 message-vote signer ready — register this address in the EOAValidator attestor set"
    );

    // Listener runs as a child task feeding us finalized messages; we sign, count, and publish.
    let (tx, mut rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let listener_provider = provider.clone();
    let listener_token = shared.token.clone();
    let confirmation_depth = cfg.block_confirmation_depth;
    // Fall back to the pre-resolution head (not "now") so the resolve-wait window is covered.
    let scan_from = cfg.start_block.or(Some(head_before_resolve));
    // Durable scan cursor: persist `last_seen` so a restart resumes exactly where it
    // left off instead of skipping down-time messages / replaying history. Scoped to the resolved
    // Outbox address so a re-registration doesn't resume against a stale one.
    let cursor_store = cursor::CursorStore::new(
        &cfg.state_dir,
        cfg.write_ability_chain_key,
        resolved.address,
    );
    tracing::info!(
        path = %cursor_store.path().display(),
        "🗂️ persisting Outbox scan cursor across restarts"
    );
    let listener_tx = tx.clone();
    let listener = tokio::spawn(async move {
        listener::watch(
            &listener_provider,
            resolved,
            confirmation_depth,
            scan_from,
            cursor_store,
            listener_tx,
            listener_token,
        )
        .await
    });

    // One live resolved-Outbox view feeds both the supervision loop below and the reobservation
    // worker. The monitor inherits the discovery cursor from initial activation, so it scans only
    // new finalized factory events — and starts over from genesis automatically when governance
    // re-points the chain key at a different factory.
    let (resolved_tx, mut resolved_rx) = watch::channel(Some(resolved));
    let mut outbox_monitor = {
        let provider = provider.clone();
        let cfg = cfg.clone();
        let token = shared.token.clone();
        let destination_chain_key = state.destination_chain_key;
        tokio::spawn(run_outbox_monitor(
            provider,
            cfg,
            destination_chain_key,
            outbox_cursor,
            resolved,
            resolved_tx,
            token,
        ))
    };
    // `listener` becomes `None` during a rotation gap (factory changed, replacement Outbox not yet
    // finalized); `active_outbox` mirrors the last value taken from the watch for log context.
    let mut listener = Some(listener);
    let mut active_outbox = Some(resolved);

    // Reobservation runs in its OWN task, NOT inline in this loop (audit P1-4): its tip + eth_getLogs
    // RPCs (deadline-bounded, serial) must not be able to stall vote production / message ingestion
    // if the shared provider black-holes. The bounded `reobs_rx` channel already drops excess, so a
    // flood is bounded to serial, deadline-capped work here.
    let mut reobs_worker = {
        let provider = provider.clone();
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
                let old_outbox = active_outbox.map(|outbox| outbox.address);
                active_outbox = next;

                if let Some(resolved) = next {
                    tracing::warn!(
                        ?old_outbox,
                        new_outbox = %resolved.address,
                        "🔄 governance/factory rotation detected — switching Outbox listener"
                    );
                    let listener_provider = provider.clone();
                    let listener_token = shared.token.clone();
                    let cursor_store = cursor::CursorStore::new(
                        &cfg.state_dir,
                        cfg.write_ability_chain_key,
                        resolved.address,
                    );
                    let listener_tx = tx.clone();
                    // Start the replacement listener at the new Outbox's creation height, not at the
                    // boot-time `scan_from`. Its cursor file is keyed by address so there is nothing
                    // persisted to resume from, and governance can point a chain key at an Outbox
                    // created *before* this process booted — whose earlier `MessagePublished` events
                    // would then sit below `scan_from` and never be scanned. Falling back to
                    // `scan_from` only when the log carried no block number keeps the old behaviour
                    // for that (not normally reachable) case.
                    let swap_start = resolved.created_at_block.or(scan_from);
                    listener = Some(tokio::spawn(async move {
                        listener::watch(
                            &listener_provider,
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
                        "⏸️ Outbox factory changed or was removed without a finalized replacement Outbox — signing paused"
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
                // Aborting the old listener does not drain what it already queued, so after a
                // rotation the channel can still hold messages observed on the superseded Outbox.
                // Signing those would contradict the pause above (and the reobservation worker,
                // which already refuses to serve requests while no Outbox is active), so gate on
                // provenance rather than on the fact that a message arrived.
                //
                // Compare against the *live* watch value, not the `active_outbox` cache: this arm
                // and the rotation arm are both ready when a rotation lands, so if this one wins
                // the cache is still one rotation behind and would wave the stale message through.
                let current = *resolved_rx.borrow();
                match current {
                    Some(active) if active.address == indexed.outbox => {
                        produce_vote(&state, &shared.metrics, &signer, our_address, chain_key, indexed);
                    }
                    Some(active) => {
                        tracing::warn!(
                            message_id = %indexed.message_id,
                            observed_on = %indexed.outbox,
                            active_outbox = %active.address,
                            "dropping a message buffered from a superseded Outbox"
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

/// Continue resolving after activation and publish a new value whenever governance points the chain
/// key at another factory or the active factory emits a replacement Outbox. A factory transition
/// with no finalized replacement Outbox publishes `None` immediately so consumers stop signing the
/// de-registered Outbox while discovery continues.
async fn run_outbox_monitor<P: Provider>(
    provider: P,
    cfg: Config,
    destination_chain_key: B256,
    mut cursor: resolver::OutboxDiscoveryCursor,
    current: resolver::ResolvedOutbox,
    resolved_tx: watch::Sender<Option<resolver::ResolvedOutbox>>,
    token: tokio_util::sync::CancellationToken,
) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(OUTBOX_RESOLVE_RETRY_SECS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut active = Some(current);
    // The factory the active Outbox was resolved from. The pause decision compares the cursor's
    // live factory against THIS — never against the cursor's value on the previous tick: `resolve`
    // records a governance transition into the cursor as soon as it reads the new registration, so
    // an attempt that then fails (RPC error / timeout) would have already consumed the transition,
    // and a tick-to-tick comparison would keep signing the superseded Outbox forever (bugbot).
    let mut active_factory = cursor.factory();
    loop {
        tokio::select! {
            () = token.cancelled() => return,
            _ = tick.tick() => {
                // Same per-attempt bound as the activation loop: an unbounded resolve against a
                // black-holed RPC would wedge rotation detection silently (the chunked cursor keeps
                // whatever progress the attempt made, so a timeout costs nothing).
                let attempt = tokio::time::timeout(
                    RPC_ATTEMPT_TIMEOUT,
                    resolver::resolve(&provider, &cfg, destination_chain_key, &mut cursor),
                )
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow!(
                        "Outbox rotation check RPC attempt timed out after {RPC_ATTEMPT_TIMEOUT:?}"
                    ))
                });
                match rotation_action(&attempt, active.map(|o| o.address), active_factory, cursor.factory()) {
                    RotationAction::Swap(next) => {
                        tracing::info!(
                            old = ?active.map(|outbox| outbox.address),
                            new = %next.address,
                            "🧭 replacement Outbox resolved"
                        );
                        active = Some(next);
                        active_factory = cursor.factory();
                        if resolved_tx.send(Some(next)).is_err() {
                            return;
                        }
                    }
                    RotationAction::KeepCurrent => {
                        active_factory = cursor.factory();
                    }
                    RotationAction::Pause => {
                        tracing::warn!(
                            old = ?active.map(|outbox| outbox.address),
                            new_factory = ?cursor.factory(),
                            "Outbox factory changed without a finalized replacement — pausing signing"
                        );
                        active = None;
                        active_factory = cursor.factory();
                        if resolved_tx.send(None).is_err() {
                            return;
                        }
                    }
                    RotationAction::Nothing => {}
                }
                if let Err(err) = attempt {
                    tracing::warn!(error = %format!("{err:#}"), "Outbox rotation check failed; will retry");
                }
            }
        }
    }
}

/// What one rotation-monitor tick should do. Pure so the consumed-transition case is testable.
#[derive(Debug, PartialEq)]
enum RotationAction {
    /// A different Outbox resolved — hot-swap the listener to it.
    Swap(resolver::ResolvedOutbox),
    /// The active Outbox re-resolved — just refresh the factory it is attributed to.
    KeepCurrent,
    /// The registered factory no longer matches the active Outbox's and no replacement has
    /// resolved — publish `None` so signing stops on the superseded Outbox.
    Pause,
    Nothing,
}

/// Pause fires on `cursor_factory != active_factory` even when the attempt ERRORED: the cursor's
/// factory only changes when `resolve` actually read a different registered factory on-chain, so a
/// scan failure after that read must not mask the transition (it would otherwise never re-fire —
/// the cursor keeps the new factory, and later ticks would see no further change).
fn rotation_action(
    attempt: &anyhow::Result<Option<resolver::ResolvedOutbox>>,
    active: Option<Address>,
    active_factory: Option<Address>,
    cursor_factory: Option<Address>,
) -> RotationAction {
    match attempt {
        Ok(Some(next)) if active != Some(next.address) => RotationAction::Swap(*next),
        Ok(Some(_)) => RotationAction::KeepCurrent,
        Ok(None) | Err(_) if active.is_some() && cursor_factory != active_factory => {
            RotationAction::Pause
        }
        Ok(None) | Err(_) => RotationAction::Nothing,
    }
}

/// Reobservation responder, run as its own task (audit P1-4). Consumes verified-on-request pull
/// requests off `reobs_rx`, re-fetches + re-signs one at a time under a wall-clock deadline, so a
/// slow/black-holed RPC can never stall the main write-ability loop (vote production + ingestion).
/// The bounded `reobs_rx` channel drops excess at the p2p ingest, so a flood is naturally bounded to
/// serial, deadline-capped work here.
#[allow(clippy::too_many_arguments)]
async fn run_reobservation_worker<P: alloy::providers::Provider>(
    provider: P,
    mut resolved_rx: watch::Receiver<Option<resolver::ResolvedOutbox>>,
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
                let handle = handle_reobservation(
                    &provider, &resolved, &state, &shared.metrics, &signer, our_address, chain_key,
                    confirmation_depth, &mut limiter, request,
                );
                // The Outbox is snapshotted above, so a rotation part-way through would have us
                // re-fetch and re-sign against the superseded address. Abandon the in-flight
                // response instead of finishing it: reobservation is a pull-based recovery path, so
                // the requester just asks again once the new listener is up, and the per-message
                // rate limiter keeps that bounded.
                tokio::select! {
                    () = shared.token.cancelled() => return,
                    changed = resolved_rx.changed() => {
                        if changed.is_err() {
                            tracing::debug!("Outbox rotation channel closed — reobservation worker exiting");
                            return;
                        }
                        tracing::warn!(
                            observed_on = %resolved.address,
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
    resolved: &resolver::ResolvedOutbox,
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

    fn outbox(addr: Address) -> resolver::ResolvedOutbox {
        resolver::ResolvedOutbox {
            address: addr,
            destination_chain_key: B256::ZERO,
            creditcoin_chain_id: 42,
            created_at_block: Some(1),
        }
    }

    const OUTBOX_A: Address = address!("00000000000000000000000000000000000000aa");
    const OUTBOX_B: Address = address!("00000000000000000000000000000000000000bb");
    const FACTORY_1: Address = address!("00000000000000000000000000000000000000f1");
    const FACTORY_2: Address = address!("00000000000000000000000000000000000000f2");

    // The bugbot case this module exists for: `resolve` records the governance transition into the
    // cursor (F1 -> F2) and then the SAME attempt fails, so the transition never coincides with a
    // clean `Ok(None)`. A tick-to-tick factory comparison consumes it; comparing against the
    // active Outbox's factory must keep firing until the pause actually happens.
    #[test]
    fn pause_survives_a_transition_consumed_by_a_failed_attempt() {
        // Tick N: cursor already moved to F2, attempt errored (scan failed after the factory read).
        let attempt: anyhow::Result<Option<resolver::ResolvedOutbox>> =
            Err(anyhow!("eth_getLogs failed"));
        assert_eq!(
            rotation_action(&attempt, Some(OUTBOX_A), Some(FACTORY_1), Some(FACTORY_2)),
            RotationAction::Pause,
        );

        // Tick N+1: clean Ok(None) — with the tick-to-tick rule this saw "no change" and kept
        // signing forever; against the active factory it still pauses.
        let attempt: anyhow::Result<Option<resolver::ResolvedOutbox>> = Ok(None);
        assert_eq!(
            rotation_action(&attempt, Some(OUTBOX_A), Some(FACTORY_1), Some(FACTORY_2)),
            RotationAction::Pause,
        );
    }

    #[test]
    fn routine_failures_and_quiet_ticks_do_nothing() {
        let err: anyhow::Result<Option<resolver::ResolvedOutbox>> = Err(anyhow!("rpc blip"));
        assert_eq!(
            rotation_action(&err, Some(OUTBOX_A), Some(FACTORY_1), Some(FACTORY_1)),
            RotationAction::Nothing,
        );
        let none: anyhow::Result<Option<resolver::ResolvedOutbox>> = Ok(None);
        assert_eq!(
            rotation_action(&none, Some(OUTBOX_A), Some(FACTORY_1), Some(FACTORY_1)),
            RotationAction::Nothing,
        );
        // Nothing active (already paused, or never resolved) — nothing to pause.
        assert_eq!(
            rotation_action(&none, None, None, Some(FACTORY_2)),
            RotationAction::Nothing,
        );
    }

    #[test]
    fn replacement_swaps_and_same_outbox_keeps_current() {
        let next = outbox(OUTBOX_B);
        let attempt: anyhow::Result<Option<resolver::ResolvedOutbox>> = Ok(Some(next));
        assert_eq!(
            rotation_action(&attempt, Some(OUTBOX_A), Some(FACTORY_1), Some(FACTORY_2)),
            RotationAction::Swap(next),
        );
        // Resuming from a pause is a swap too (nothing was active).
        assert_eq!(
            rotation_action(&attempt, None, None, Some(FACTORY_2)),
            RotationAction::Swap(next),
        );
        // Same address re-resolved (e.g. a new factory adopting the same Outbox): keep signing,
        // but re-attribute the active Outbox to the cursor's factory.
        let same: anyhow::Result<Option<resolver::ResolvedOutbox>> = Ok(Some(outbox(OUTBOX_A)));
        assert_eq!(
            rotation_action(&same, Some(OUTBOX_A), Some(FACTORY_1), Some(FACTORY_2)),
            RotationAction::KeepCurrent,
        );
    }
}
