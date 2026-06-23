//! USC write-ability: cross-chain message attestation (confluence §7.3).
//!
//! When enabled (`message_attestation_enabled`), this task makes the attestor a **message
//! validator**: it watches the Creditcoin L1 Outbox for its `chain_key`, signs the canonical
//! `messageHash` of each finalized `MessagePublished`, and gossips an ECDSA [`MessageVote`] on
//! `{chain_key}/message-votes/v1`. Relayers snoop the same topic and deliver once 2/3+1 unique
//! attesters have voted — the attestor never relays or touches the destination chain (§1).
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
pub mod config;
pub mod ingest;
pub mod listener;
pub mod resolver;
pub mod signing;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use alloy::primitives::Address;
use alloy::providers::ProviderBuilder;
use anyhow::anyhow;
use parking_lot::Mutex;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use write_ability::envelope::MessageVote;

use crate::error::Error;
use crate::shared::Shared;

pub use config::{AttesterSet, Config};

/// Message-vote state shared between this task (producer) and the p2p task (publisher + incoming
/// validator). Lives on [`Shared`](crate::shared::Shared) as `Option`, set only when message
/// attestation is enabled with a usable attester set.
pub struct MessageVoteState {
    /// In-memory vote aggregator (chain-first allowlist, dedup, threshold, anti-abuse caps).
    pub aggregator: Mutex<aggregator::VoteAggregator>,
    /// Authorized signer EVM addresses; gossip votes from outside this set are rejected.
    pub active_set: HashSet<Address>,
    /// Outgoing votes we produced, handed to the p2p task to publish on the message-vote topic.
    pub publish_tx: mpsc::Sender<MessageVote>,
}

/// Build the shared message-vote state and the matching publish channel receiver from config, or
/// `None` when message attestation is disabled / not yet supported. Pure (no async) so it can run
/// during `lib.rs` startup before tasks spawn. Called from `lib.rs`.
#[must_use]
pub fn build_state(cfg: &Config) -> Option<(Arc<MessageVoteState>, mpsc::Receiver<MessageVote>)> {
    if !cfg.enabled {
        return None;
    }
    let active_set = match &cfg.attester_set {
        AttesterSet::Static(addrs) if !addrs.is_empty() => {
            addrs.iter().copied().collect::<HashSet<Address>>()
        }
        AttesterSet::Static(_) => {
            tracing::error!("message attestation enabled but attester_set is empty — disabling");
            return None;
        }
        AttesterSet::OnChainValidator(addr) => {
            tracing::error!(
                %addr,
                "on-chain validator attester set is not yet wired into the attestor; \
                 configure a static attester_set — disabling message attestation"
            );
            return None;
        }
    };
    let threshold = attestor_primitives::calculate_threshold(active_set.len() as u32) as usize;
    let aggregator =
        aggregator::VoteAggregator::new(threshold, cfg.max_tracked_messages, cfg.vote_ttl);
    let (publish_tx, publish_rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let state = Arc::new(MessageVoteState {
        aggregator: Mutex::new(aggregator),
        active_set,
        publish_tx,
    });
    tracing::info!(
        attesters = state.active_set.len(),
        threshold,
        "🧑‍🤝‍🧑 message-vote quorum configured"
    );
    Some((state, publish_rx))
}

/// Entry point spawned from `lib.rs`. Drives the Outbox listener and produces signed votes; the
/// swarm itself is owned by the p2p task. `seed` is the 32-byte secret the EVM key derives from.
pub async fn run(shared: Arc<Shared>, cfg: Config, seed: Zeroizing<[u8; 32]>) -> Result<(), Error> {
    let Some(state) = shared.message_votes.clone() else {
        tracing::info!("📭 message attestation disabled — parking write-ability task");
        // Park until shutdown; returning Ok early would trip the supervisor's "exited early" guard.
        shared.token.cancelled().await;
        return Ok(());
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

    let resolved = resolver::resolve(&provider, shared.chain_key, &cfg)
        .await
        .map_err(Error::WriteAbility)?;

    let signer = signing::MessageSigner::from_seed(&seed).map_err(Error::WriteAbility)?;
    let our_address = signer.address();
    tracing::info!(
        evm_address = %our_address,
        "🔑 message-vote signer ready — register this address in the EOAValidator attester set"
    );

    // Listener runs as a child task feeding us finalized messages; we sign, count, and publish.
    let (tx, mut rx) = mpsc::channel(common::constants::CAPACITY_CHANNEL);
    let listener_provider = provider.clone();
    let listener_token = shared.token.clone();
    let confirmation_depth = cfg.block_confirmation_depth;
    let listener = tokio::spawn(async move {
        if let Err(err) = listener::watch(
            &listener_provider,
            resolved,
            confirmation_depth,
            tx,
            listener_token,
        )
        .await
        {
            tracing::error!(%err, "outbox listener exited with error");
        }
    });

    let chain_key = shared.chain_key;
    loop {
        tokio::select! {
            biased;
            () = shared.token.cancelled() => break,
            maybe = rx.recv() => {
                let Some(indexed) = maybe else {
                    tracing::warn!("listener channel closed — write-ability task exiting");
                    break;
                };
                produce_vote(&state, &signer, our_address, chain_key, indexed);
            }
        }
    }

    listener.abort();
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
