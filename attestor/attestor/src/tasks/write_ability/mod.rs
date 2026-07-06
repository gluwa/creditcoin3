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
pub mod ingest;
pub mod listener;
pub mod reobservation;
pub mod resolver;
pub mod signing;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use alloy::primitives::{Address, B256};
use alloy::providers::ProviderBuilder;
use anyhow::anyhow;
use parking_lot::{Mutex, RwLock};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use write_ability::envelope::{MessageVote, ReobservationRequest};
use write_ability::protocol::chain_key_to_bytes32;

use crate::error::Error;
use crate::shared::Shared;

pub use config::{AttestorSet, Config};

/// How often to re-attempt Outbox resolution while it is not yet registered on-chain (dynamic
/// activation without a restart).
const OUTBOX_RESOLVE_RETRY_SECS: u64 = 12;

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
)> {
    if !cfg.enabled {
        return None;
    }
    let destination_chain_key = resolve_destination_chain_key(cfg, cc3).await?;
    let active_set = resolve_active_set(cfg).await?;
    let threshold = attestor_primitives::calculate_threshold(active_set.len() as u32) as usize;
    let aggregator =
        aggregator::VoteAggregator::new(threshold, cfg.max_tracked_messages, cfg.vote_ttl);
    let (publish_tx, publish_rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let (reobs_tx, reobs_rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let state = Arc::new(MessageVoteState {
        aggregator: Mutex::new(aggregator),
        active_set: RwLock::new(active_set),
        publish_tx,
        reobs_tx,
        destination_chain_key,
    });
    tracing::info!(
        attestors = state.active_set.read().len(),
        threshold,
        "🧑‍🤝‍🧑 message-vote quorum configured"
    );
    Some((state, publish_rx, reobs_rx))
}

/// Read the on-chain `WriteAbilityConfigs` entry for this `chain_key` and derive the effective
/// `bytes32` write-ability chain key. Returns `None` when governance has explicitly disabled
/// message attestation for the chain (an entry exists with `message_attestation_enabled == false`).
async fn resolve_destination_chain_key(cfg: &Config, cc3: &cc_client::Client) -> Option<B256> {
    let chain_key = cfg.write_ability_chain_key;
    let local = chain_key_to_bytes32(chain_key);
    match cc3.get_write_ability_config(chain_key).await {
        Ok(Some(on_chain)) => {
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
        Ok(None) => {
            tracing::warn!(
                chain_key,
                "no on-chain WriteAbilityConfig registered for this chain — using the locally derived chain key"
            );
            Some(local)
        }
        Err(err) => {
            // Availability over strictness: a transient read failure must not disable a locally
            // configured attestor. Explicit governance "off" is only honored via Ok(Some(..)).
            tracing::warn!(
                chain_key,
                %err,
                "failed to read on-chain WriteAbilityConfig — falling back to local config"
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
            let Some(url) = cfg.destination_eth_rpc_url.as_ref() else {
                tracing::error!(
                    %validator,
                    "OnChainValidator attestor set configured but no destination_eth_rpc_url — disabling"
                );
                return None;
            };
            let set = match attestor_set::fetch_attestor_set(url.as_str(), *validator).await {
                Ok(set) => set,
                Err(err) => {
                    tracing::error!(%validator, %err, "failed to read attestor set from on-chain EOAValidator — disabling");
                    return None;
                }
            };
            if set.is_empty() {
                tracing::error!(%validator, "EOAValidator.attestors() returned an empty set — disabling");
                return None;
            }
            tracing::info!(
                %validator,
                attestors = set.len(),
                "🧑‍⚖️ read attestor set from on-chain EOAValidator"
            );
            Some(set)
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
    mut reobs_rx: mpsc::Receiver<ReobservationRequest>,
) -> Result<(), Error> {
    let Some(state) = shared.message_votes.clone() else {
        tracing::info!("📭 message attestation disabled — parking write-ability task");
        // Park until shutdown; returning Ok early would trip the supervisor's "exited early" guard.
        shared.token.cancelled().await;
        return Ok(());
    };

    // On-chain attestor-set hot-reload watcher (only when the set is sourced from the validator).
    // Runs independently of Outbox resolution — the set is unrelated to the Outbox — so it keeps the
    // active set in sync even while write-ability is idle waiting for the Outbox.
    let set_watcher = match (&cfg.attestor_set, cfg.destination_eth_rpc_url.as_ref()) {
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

    let rpc = cfg.cc3_eth_rpc_url.as_ref().ok_or_else(|| {
        Error::WriteAbility(anyhow!(
            "cc3_eth_rpc_url is required when message attestation is on"
        ))
    })?;
    let provider = ProviderBuilder::new()
        .on_builtin(rpc.as_str())
        .await
        .map_err(|e| Error::WriteAbility(anyhow!("connect Creditcoin L1 EVM RPC: {e}")))?;

    // Resolve the Outbox, retrying until it's available rather than disabling for the whole run:
    // an attestor can be started before the factory/Outbox is registered on-chain and will activate
    // write-ability automatically once they are, with no restart. While unresolved it just keeps
    // doing block attestation. (Polling is simpler and more robust than event subscription; picking
    // up a later Outbox *re-registration* mid-run remains a finer-grained TODO in resolver.rs.)
    let resolved = loop {
        match resolver::resolve(&provider, &cfg, state.destination_chain_key).await {
            Ok(Some(r)) => break r,
            Ok(None) => {
                tracing::info!(
                    retry_secs = OUTBOX_RESOLVE_RETRY_SECS,
                    "📭 no Outbox factory/Outbox registered on-chain yet — write-ability idle; will retry"
                );
            }
            Err(err) => {
                tracing::warn!(%err, retry_secs = OUTBOX_RESOLVE_RETRY_SECS, "Outbox resolution failed — will retry");
            }
        }
        tokio::select! {
            () = shared.token.cancelled() => return Ok(()),
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
    let mut listener = tokio::spawn(async move {
        listener::watch(
            &listener_provider,
            resolved,
            confirmation_depth,
            cfg.start_block,
            tx,
            listener_token,
        )
        .await
    });

    // Cooldown so a spammed/forged reobservation topic can't make us re-scan the chain in a loop.
    let mut reobs_limiter = reobservation::ReobsRateLimiter::new(reobservation::REOBS_MIN_INTERVAL);
    // Flipped off if the reobservation sender is ever dropped, so we stop polling a closed channel.
    let mut reobs_open = true;

    let chain_key = shared.chain_key;
    loop {
        tokio::select! {
            biased;
            () = shared.token.cancelled() => break,
            maybe = rx.recv() => {
                let Some(indexed) = maybe else {
                    // The listener holds the only sender, so a closed channel means it exited.
                    // Harvest its result and surface the underlying error to the supervisor —
                    // otherwise it would only see a generic early-Ok exit and the failure reason
                    // would be lost.
                    let err = match (&mut listener).await {
                        Ok(Ok(())) => anyhow!("outbox listener exited without error or shutdown"),
                        Ok(Err(err)) => err.context("outbox listener died"),
                        Err(join_err) => anyhow!("outbox listener panicked: {join_err}"),
                    };
                    if let Some(w) = &set_watcher {
                        w.abort();
                    }
                    return Err(Error::WriteAbility(err));
                };
                produce_vote(&state, &signer, our_address, chain_key, indexed);
            }
            maybe = reobs_rx.recv(), if reobs_open => {
                let Some(request) = maybe else {
                    reobs_open = false;
                    continue;
                };
                handle_reobservation(
                    &provider, &resolved, &state, &signer, our_address, chain_key,
                    &mut reobs_limiter, request,
                ).await;
            }
        }
    }

    listener.abort();
    if let Some(w) = set_watcher {
        w.abort();
    }
    Ok(())
}

/// Sign our vote for a freshly indexed message, count it locally (chain-seen + our own signature),
/// and hand it to the p2p task to gossip.
fn produce_vote(
    state: &MessageVoteState,
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

    // Chain-seen (we observed it on-chain) + count our own vote.
    {
        let now = Instant::now();
        let mut agg = state.aggregator.lock();
        agg.note_indexed(indexed.message_hash.0, now);
        if let aggregator::VoteOutcome::Accepted {
            reached_threshold: true,
        } = agg.add_vote(indexed.message_hash.0, our_address, now)
        {
            ingest::note_threshold(chain_key, &indexed.message_hash);
        }
    }

    let vote = MessageVote {
        chain_key,
        message_id: indexed.message_id.0,
        message_hash: indexed.message_hash.0,
        signer: our_address.into_array(),
        signature,
    };

    match state.publish_tx.try_send(vote) {
        Ok(()) => tracing::info!(
            message_id = %indexed.message_id,
            message_hash = %indexed.message_hash,
            "✉️ queued message vote for gossip"
        ),
        Err(err) => {
            tracing::warn!(%err, "message-vote publish channel full/closed — dropping vote")
        }
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
    signer: &signing::MessageSigner,
    our_address: Address,
    chain_key: u64,
    limiter: &mut reobservation::ReobsRateLimiter,
    request: ReobservationRequest,
) {
    let message_id = alloy::primitives::B256::from(request.message_id);
    if request.chain_key != chain_key {
        return; // not ours (topic is per-chain, but be defensive)
    }
    if !limiter.allow(message_id, Instant::now()) {
        tracing::debug!(%message_id, "⏳ reobservation request within cooldown — ignoring");
        return;
    }

    let indexed = match reobservation::reobserve(provider, resolved, &request).await {
        Ok(Some(indexed)) => indexed,
        Ok(None) => {
            tracing::warn!(
                %message_id,
                block = request.block_height,
                "🔎 reobservation request did not match a verifiable MessagePublished — ignoring"
            );
            return;
        }
        Err(err) => {
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
    produce_vote(state, signer, our_address, chain_key, indexed);
}
