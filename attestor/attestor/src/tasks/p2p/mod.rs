//! libp2p gossip task.
//!
//! Differences from v1:
//!
//! - Gossips **lightweight votes** (`attestor_pool::Vote`), not full attestations.
//! - Receives outgoing votes via an `mpsc::Receiver<Vote>`, not a broadcast channel
//!   (we have one consumer, not many).
//! - Verifies incoming votes via the local `proof_cache` (peers must have produced their own
//!   AttestationData at that height to verify a remote BLS signature).

pub(crate) mod admission;
pub mod behavior;
pub mod protocols;

use std::sync::Arc;
use std::time::Instant;

use parity_scale_codec::{DecodeAll, Encode};
use tokio::sync::mpsc;

use attestor_pool::Vote;
use attestor_primitives::AttestorId;
use write_ability::envelope::{ReobservationRequest, SetUpdateVote};
use write_ability::protocol::{
    attestor_set_update_topic, message_votes_topic, reobservation_topic,
};

use crate::error::Error;
use crate::shared::Shared;
use crate::tasks::write_ability::reobservation::{
    CooldownMap, QueuePush, ReobsKey, ReobsQueue, REOBS_FAILURE_INTERVAL,
};
use crate::tasks::write_ability::{ingest, pending};
use crate::vote::{verify_vote, VerifyResult};

/// Consecutive failed pings on a single connection before we reap it.
const MAX_PING_FAILURES: u32 = 3;

/// Consecutive failed *dials* to a peer before we drop it from the Kademlia routing table.
/// Kademlia never evicts on dial failure by itself, and peers re-share stale records with each
/// other, so a node that shut down while still registered on-chain would otherwise be redialed
/// forever (the deny-list only covers attestors that were chilled/kicked). Eviction is safe: if
/// the peer comes back it re-announces via identify/kad discovery and is re-added as usual.
const MAX_DIAL_FAILURES: u32 = 5;

/// Upper bound on locally-produced votes retained in the re-gossip outbox (see `outbox` in
/// [`run`]). Overflow drops the *oldest* entry — the height most likely to finalize without our
/// broadcast anyway.
const MAX_OUTBOX: usize = 256;

/// Upper bound on an inbound attestor-set-update frame we'll relay. A `SetUpdateVote` is a bounded
/// attestor set (≤ [`common::constants::MAX_ATTESTORS`] 20-byte addresses) plus one 65-byte
/// signature, a chain id and a nonce, under SCALE framing — comfortably below this. Larger frames
/// on that topic are not real votes; we `Ignore` them (no propagation, no peer penalty) instead of
/// amplifying. Well under gossipsub's 64 KiB default transmit cap.
const MAX_SET_UPDATE_FRAME_BYTES: usize = common::constants::MAX_ATTESTORS * 20 + 1024;

/// How often unfinalized local votes are re-gossiped while the outbox is non-empty. Re-sends also
/// fire immediately when gossip publishing first becomes possible again (mesh regained).
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(builder::Builder)]
pub struct Config {
    pub boot_nodes: Vec<libp2p::Multiaddr>,
    /// Trusted committee peer IDs, admitted before any vote binds an attestor to a peer.
    /// These reserve inbound capacity only; discovery and BLS vote verification are unchanged.
    #[default(Vec::new())]
    pub reserved_peers: Vec<libp2p::PeerId>,
    pub public_addr: Option<String>,
    pub port: u16,
    #[default(false)]
    pub no_mdns: bool,

    #[specify_later]
    pub keypair: libp2p::identity::Keypair,
    #[specify_later]
    pub chain_key: attestor_primitives::ChainKey,
}

pub async fn run(
    shared: Arc<Shared>,
    cfg: Config,
    mut gossip_rx: mpsc::UnboundedReceiver<Vote>,
    mut peer_deactivated_rx: mpsc::UnboundedReceiver<crate::shared::PeerDeactivated>,
    mut mv_publish_rx: mpsc::Receiver<pending::PendingVote>,
    mut set_update_publish_rx: mpsc::Receiver<SetUpdateVote>,
    mut peer_registry_rx: tokio::sync::watch::Receiver<Arc<crate::peer_registry::Registry>>,
) -> Result<(), Error> {
    use futures::StreamExt as _;

    let enable_mdns = !cfg.no_mdns;
    let chain_key = cfg.chain_key;
    let mut peer_registry = peer_registry_rx.borrow_and_update().clone();
    let mut minimum_registry_number = peer_registry.finalized_number;

    // Built before the swarm because the behavior needs the topic hash to install its
    // gossipsub peer-scoring topic parameters.
    let topic = libp2p::gossipsub::IdentTopic::new(format!("{chain_key}/attest"));

    // The write-ability topics are built HERE too, for the same reason, even though they are subscribed
    // further down. Gossipsub peer scoring is per-topic and can only be installed once, at behaviour
    // construction: `PeerScoreParams::topics` is a map, and `mark_invalid_message_delivery` no-ops for a
    // topic missing from it. A topic subscribed after scoring is installed is therefore *unscored*, which
    // silently turns `report_message_validation_result(.., Reject)` on that topic into a no-op — the
    // flooder pays nothing (audit finding). Building them up front lets the behaviour score every topic
    // we will ever subscribe to. They depend only on `chain_key` and whether message attestation is
    // enabled, both known here.
    //
    // Keep these in lockstep with the `subscribe` calls below: subscribing to a topic without adding it
    // to `credited_topics` or `penalty_only_topics` reintroduces the finding.
    let mv_topic = shared
        .message_votes
        .is_some()
        .then(|| libp2p::gossipsub::IdentTopic::new(message_votes_topic(chain_key)));
    let reobs_topic = shared
        .message_votes
        .is_some()
        .then(|| libp2p::gossipsub::IdentTopic::new(reobservation_topic(chain_key)));
    let set_update_topic = shared
        .message_votes
        .is_some()
        .then(|| libp2p::gossipsub::IdentTopic::new(attestor_set_update_topic(chain_key)));

    // Split by whether an `Accept` on the topic means the frame passed real validation. Per-topic scores
    // are SUMMED into the single peer score the thresholds apply to, so positive credit on one topic
    // offsets penalties on another. Two topics accept without validating and so earn no credit:
    //
    // * set-update — `handle_swarm` accepts (and propagates) any frame under a size bound without
    //   decoding it at all.
    // * reobservation — `handle_reobservation_request` returns `Accept` as soon as the frame decodes
    //   and its `chain_key` matches. `ReobservationRequest` is four unsigned plain fields, so there is
    //   nothing to forge: any peer can emit well-formed requests with random message ids. The RPC
    //   verification happens later, in `run_reobservation_worker`, and its verdict never reaches
    //   `report_message_validation_result`. (The per-source rate limiter bounds the farming rate,
    //   since it runs before decode and returns `Ignore`, but not the principle.)
    //
    // Crediting either would let a peer farm P2 first-message-delivery score at zero cost up to the
    // P2 cap, then spend that buffer absorbing P4 penalties earned on the topics that DO validate.
    let credited_topics: Vec<&libp2p::gossipsub::IdentTopic> =
        std::iter::once(&topic).chain(mv_topic.as_ref()).collect();
    let penalty_only_topics: Vec<&libp2p::gossipsub::IdentTopic> = set_update_topic
        .as_ref()
        .into_iter()
        .chain(reobs_topic.as_ref())
        .collect();
    tracing::debug!(
        credited = credited_topics.len(),
        penalty_only = penalty_only_topics.len(),
        "🛡️ installing gossipsub peer-score params for every subscribed topic"
    );

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(cfg.keypair)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )
        .map_err(|e| Error::P2p(e.into()))?
        .with_quic()
        .with_dns()
        .map_err(|e| Error::P2p(e.into()))?
        .with_behaviour(|k| {
            behavior::P2PBehavior::new(k, enable_mdns, &credited_topics, &penalty_only_topics)
        })
        .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?
        // libp2p's default idle_connection_timeout is ZERO: a connection holding no active
        // protocol stream is torn down immediately. Attestation topics are per-chain, so a
        // connection to an attestor serving a *different* chain never carries a gossipsub
        // stream — under the default it closes the instant identify finishes, mdns rediscovers
        // the peer, and the pair redials forever (observed in CI: ~300 reconnects per peer over
        // 28 minutes, destabilizing the local mesh during the genesis window). A real timeout
        // lets such connections sit idle instead of thrashing.
        .with_swarm_config(|c| c.with_idle_connection_timeout(std::time::Duration::from_secs(60)))
        .build();

    tracing::info!(%topic, "📫 subscribing to lightweight attestation gossip");
    swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic)
        .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;

    // Configured boot nodes are exempt from dial-failure eviction below: they are the rendezvous
    // points the mesh reforms through, so a temporarily-down bootnode must keep being retried
    // rather than dropped from the routing table.
    let mut boot_peers: std::collections::HashSet<libp2p::PeerId> =
        std::collections::HashSet::new();

    // Write-ability piggybacks on this same swarm: when message attestation is enabled we subscribe
    // to the message-vote topic too (same peers / discovery), and the dispatch in `handle_swarm`
    // routes frames by topic. `None` when disabled — no extra subscription, no behaviour change.
    // (The topic itself was built before the swarm so peer scoring covers it — see above.)
    if let Some(mv_topic) = &mv_topic {
        tracing::info!(topic = %mv_topic, "📫 subscribing to message-vote gossip");
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(mv_topic)
            .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;
    }

    // Reobservation requests (liveness recovery) ride the same swarm on their own topic. We only
    // *receive* these (relayers publish them); on a valid request we re-verify + re-sign. Subscribed
    // alongside the vote topic so message attestation is fully enabled or fully off.
    if let Some(reobs_topic) = &reobs_topic {
        tracing::info!(topic = %reobs_topic, "📫 subscribing to reobservation requests");
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(reobs_topic)
            .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;
    }

    // Attestor-set-update votes (P2-8) ride the same swarm on their own topic. We only *publish*
    // these (the proposer produces them; the relayer consumes them). Subscribing is required to
    // publish/propagate. Gated on message attestation like the topics above.
    if let Some(set_update_topic) = &set_update_topic {
        tracing::info!(topic = %set_update_topic, "📫 subscribing to attestor-set-update gossip");
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(set_update_topic)
            .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;
    }

    let local_peer_id = *swarm.local_peer_id();
    for address in cfg.boot_nodes {
        let Some(peer_id) = address.iter().find_map(|p| match p {
            libp2p::multiaddr::Protocol::P2p(pid) => Some(pid),
            _ => None,
        }) else {
            tracing::error!(%address, "missing peer id in multiaddr");
            continue;
        };
        if peer_id == local_peer_id {
            tracing::warn!(%address, "ignoring own peer id in boot node multiaddr");
            continue;
        }
        boot_peers.insert(peer_id);
        swarm.behaviour_mut().kad.add_address(&peer_id, address);
    }

    // Reserve trusted inbound peers before listening. A seed with no boot peers cannot rely on
    // learning committee bindings from votes: Sybils may fill the unreserved pool first, leaving
    // legitimate attestors unable to connect and send the very vote that would reserve them.
    swarm.behaviour_mut().admission.set_committee_peers(
        peer_registry
            .registered_peers()
            .filter(|peer| *peer != local_peer_id),
        peer_registry.registration_required,
    );
    configure_bootstrap_admission(
        &mut swarm.behaviour_mut().admission,
        &boot_peers,
        &cfg.reserved_peers,
        local_peer_id,
        enable_mdns,
        cfg.public_addr.is_some(),
        &peer_registry,
    )
    .map_err(Error::P2p)?;

    if let Some(dns) = cfg.public_addr {
        let external: libp2p::Multiaddr = format!("/dns4/{}/tcp/{}", dns, cfg.port)
            .parse()
            .map_err(|e: libp2p::multiaddr::Error| Error::P2p(e.into()))?;
        tracing::info!(%external, "📰 broadcasting external address");
        swarm.add_external_address(external);
    }

    let listen_addr: libp2p::Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", cfg.port)
        .parse()
        .map_err(|e: libp2p::multiaddr::Error| Error::P2p(e.into()))?;
    swarm
        .listen_on(listen_addr.clone())
        .map_err(|e| Error::P2p(e.into()))?;

    // Mesh-visibility hints: "≥1 gossipsub mesh peer on <topic>", recomputed after every swarm
    // event. Used ONLY as edge triggers for eagerly flushing the matching retry queue when that
    // topic's mesh (re)forms — never as a gate on publishing. Gating publishes on a hint wedged
    // the node whenever the mesh regained peers without a swarm event (heartbeat GRAFTs, score
    // recovery): the flag stayed false, both publish paths were short-circuited, and the node
    // never even attempted to publish again. `try_publish`'s own error is the authoritative "no
    // peers" signal (and with flood_publish, topic peers — not mesh membership — are what
    // publishing actually needs).
    //
    // One hint per topic: gossipsub meshes are independent per topic and form at different times,
    // so keying the message-vote flush off the attest topic's mesh would fire it into a peerless
    // mv mesh (harmless — stays queued) or miss the mv mesh forming first (flush deferred to the
    // periodic retry tick).
    let mut can_broadcast = false;
    let mut can_broadcast_mv = false;

    // Per-height pending buffer for incoming votes that arrived before our local production
    // reached that height. Drained when `shared.local_produced_rx` changes.
    //
    // Keyed by attestor within each height so a peer flooding garbage votes that *claim* active
    // attestor identities can occupy at most one slot per claimed attestor — it can no longer
    // push legitimate early votes out of a shared per-height queue (one-vote-per-attestor holds
    // even before BLS verification is possible).
    //
    // The per-height cap must cover the *whole* committee: a lagging node can receive every peer's
    // vote for a height before it produces local data, and quorum for a full committee is well
    // above any smaller cap. Spillover is dropped with `Acceptance::Ignore`, and under gossipsub
    // Strict validation an `Ignore`d message stays in the seen-cache and is *never* redelivered —
    // so a dropped vote is lost to this node permanently and the height can miss quorum. Sizing
    // from `MAX_ATTESTORS` (the runtime committee bound) guarantees we never drop a distinct
    // attestor's early vote, while the per-attestor keying still bounds memory to the committee.
    const MAX_PENDING_PER_HEIGHT: usize = common::constants::MAX_ATTESTORS;
    let mut pending_votes: PendingVotes = std::collections::HashMap::new();

    // Modern libp2p ping no longer tears down a connection on repeated failures; we do it
    // ourselves. Tracks consecutive failed pings per connection and reaps the connection once it
    // crosses MAX_PING_FAILURES so a wedged socket can't silently starve the mesh.
    let mut ping_failures: std::collections::HashMap<libp2p::swarm::ConnectionId, u32> =
        std::collections::HashMap::new();

    // Consecutive failed dial attempts per peer (transport-level failures only — timeouts,
    // refused, handshake errors). Cleared the moment any connection to the peer is established,
    // so only genuinely unreachable peers accumulate towards MAX_DIAL_FAILURES. Bounded by the
    // set of peers we actually dial (committee-sized), and entries are removed on eviction.
    let mut dial_failures: std::collections::HashMap<libp2p::PeerId, u32> =
        std::collections::HashMap::new();

    // Finalized controller → networking identity bindings take priority over gossip. During
    // rollout, BLS-verified votes may supply a temporary binding only for active controllers
    // with no registered key. Retained inactive bindings support eviction and a bounded deny
    // history; a controller's new finalized key replaces its previous binding.
    let mut peers_by_attestor: Vec<(AttestorId, libp2p::PeerId)> = Vec::new();
    apply_peer_registry(
        &mut swarm,
        &mut peers_by_attestor,
        &peer_registry,
        &cfg.reserved_peers,
        &mut minimum_registry_number,
    );
    ensure_local_peer_key(&peer_registry, &shared.attestor_id, local_peer_id)
        .map_err(Error::P2p)?;

    // Per-peer + global rate limit for inbound reobservation requests (audit P1-4).
    let mut reobs_admission = ReobsAdmission::new(Instant::now());

    let mut local_produced_rx = shared.local_produced_rx.clone();
    let mut latest_finalized_rx = shared.latest_finalized_rx.clone();

    // Outbox of locally-produced votes, re-gossiped on a fixed interval (and when the mesh
    // re-forms) until their height finalizes on-chain. A vote used to leave the queue on publish
    // *success* — but gossipsub returning Ok only means the message was handed to ≥1 connection,
    // not that any peer received it. During the CI genesis deadlock all three chain-4 attestors
    // "successfully" published their genesis vote seconds after boot, every copy evaporated in
    // connection churn, and with no re-send the 3-vote quorum could never assemble: each attestor
    // waited on BlockAttested(genesis) forever while anvil kept mining. Keeping votes queued
    // until *finalization* (pruned by the `latest_finalized_rx` arm below) makes any single lost
    // publish self-healing at the next tick instead of permanent.
    let mut outbox: std::collections::VecDeque<Vote> = std::collections::VecDeque::new();
    // Retry queue for locally signed message votes: a vote produced while the message-votes mesh
    // has no peers must not be lost — the relayer can only count votes it hears. Bounded like the
    // outbox; message votes keep publish-failure-retry semantics (not re-gossip-until-finalized):
    // they have no on-chain finalization to prune against, stale entries age out of the relayer's
    // aggregation window harmlessly, and the cap evicts the oldest first.
    let mut mv_retry_queue: std::collections::VecDeque<pending::PendingVote> =
        std::collections::VecDeque::new();
    let mut retry_tick = tokio::time::interval(RETRY_INTERVAL);
    retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = shared.token.cancelled() => return Ok(()),

            result = peer_registry_rx.changed() => {
                if result.is_err() {
                    return Err(Error::P2p(anyhow::anyhow!("finalized peer registry task stopped")));
                }
                let next = peer_registry_rx.borrow_and_update().clone();
                if next.finalized_number < minimum_registry_number {
                    tracing::debug!(observed = next.finalized_number, minimum = minimum_registry_number,
                        "ignoring registry older than observed membership removal");
                    continue;
                }
                ensure_local_peer_key(&next, &shared.attestor_id, local_peer_id).map_err(Error::P2p)?;
                update_peer_registry(&mut swarm, &mut peers_by_attestor, &peer_registry, &next,
                    &cfg.reserved_peers, &mut minimum_registry_number);
                peer_registry = next;
            }

            // Outgoing — production gives us a freshly built local vote to gossip. Always drain
            // the channel (even while peerless): the channel is unbounded, so draining
            // unconditionally is what keeps it from growing; anything unpublishable goes to the
            // bounded retry queue. Always *attempt* the publish — `try_publish` failing is the
            // authoritative no-peers signal; pre-gating on the mesh hint wedged publishing
            // whenever the hint went stale-false (see `can_broadcast`).
            Some(vote) = gossip_rx.recv() => {
                // Publish eagerly, but queue regardless of the result: Ok is not delivery (see
                // the outbox comment above), so the vote stays until its height finalizes.
                let solo = shared.bls_store.is_sole_member(shared.attestor_id.account_id());
                try_publish(&mut swarm, &topic, &vote, solo);
                enqueue_unfinalized(&mut outbox, vote);
            }

            // Periodic re-gossip of unfinalized block votes plus retry of unpublished message
            // votes. Unconditional attempts — a publish failing is the authoritative no-peers
            // signal; each sweep stops at its first failure, so a peerless tick costs one publish
            // call per queue per 30s. Steady state keeps this cheap: only in-flight heights stay
            // in the outbox, and peers' pools dedup repeats by (height, attestor).
            _ = retry_tick.tick(), if !outbox.is_empty() || !mv_retry_queue.is_empty() => {
                let solo = shared.bls_store.is_sole_member(shared.attestor_id.account_id());
                republish_outbox(&mut outbox, |vote| try_publish(&mut swarm, &topic, vote, solo));
                if let Some(mv_topic) = &mv_topic {
                    flush_mv_retry_queue(&mut swarm, mv_topic, &mut mv_retry_queue, shared.message_votes.as_ref().expect("message topic has state"));
                }
            }

            // Local production cached new AttestationData → drain every buffered height up to
            // the latest notification that now has local data. `watch` coalesces updates, so a
            // catch-up burst may wake us at height N after production also emitted N-1, N-2, ...
            // Draining only N would silently discard those now-verifiable votes.
            res = local_produced_rx.changed() => {
                if res.is_err() { return Ok(()); }
                let Some(h) = *local_produced_rx.borrow() else { continue; };
                let ready = drain_pending_votes(&mut pending_votes, h, |height| {
                    shared.proof_cache.local_data(height).is_some()
                });
                for vote in ready {
                    retry_pending_vote(&shared, vote).await;
                }
            }

            // An attestation finalized on chain → drop every buffered vote at or below it. Bounds
            // the buffer even when our local production schedule never reaches the heights a peer
            // buffered (e.g. it stalled, or the votes were just shy of producible). Unpublished
            // votes at or below the finalized height no longer need propagation either.
            res = latest_finalized_rx.changed() => {
                if res.is_err() { return Ok(()); }
                let finalized = latest_finalized_rx.borrow().map(|info| info.height);
                if let Some(fin) = finalized {
                    pending_votes.retain(|&height, _| height > fin);
                    outbox.retain(|vote| vote.height > fin);
                }
            }

            // Production observed a chill/kick on chain and refreshed the active set — evict the
            // deactivated attestor's peer (if we know it) and keep it out until it reactivates.
            Some(event) = peer_deactivated_rx.recv() => {
                handle_peer_deactivated(
                    &mut swarm,
                    &mut peers_by_attestor,
                    &peer_registry,
                    &mut minimum_registry_number,
                    &event,
                    |id| shared.bls_store.pubkey(id.account_id()).is_some(),
                );
            }

            // Outgoing — the write_ability task produced a signed message vote to gossip. A
            // publish that fails (typically no mesh peers yet) is queued and retried, same as
            // block-attestation votes: dropping it would silently cost the relayer our signature.
            Some(vote) = mv_publish_rx.recv(), if mv_topic.is_some() => {
                if let Some(mv_topic) = &mv_topic {
                    if !try_publish_message_vote(&mut swarm, mv_topic, &vote, shared.message_votes.as_ref().expect("message topic has state")) {
                        queue_message_vote_for_retry(&mut mv_retry_queue, vote);
                    }
                }
            }

            // Outgoing — the proposer produced a signed attestor-set-update vote. Unlike message
            // votes there is no retry queue: the proposer re-emits every poll while the set stays
            // diverged, so a failed publish (e.g. no mesh peers yet) self-heals on the next cycle.
            Some(vote) = set_update_publish_rx.recv(), if set_update_topic.is_some() => {
                if let Some(set_update_topic) = &set_update_topic {
                    try_publish_set_update_vote(&mut swarm, set_update_topic, &vote);
                }
            }

            // Incoming events from the swarm.
            event = swarm.select_next_some() => {
                let could_broadcast = can_broadcast;
                let could_broadcast_mv = can_broadcast_mv;
                handle_swarm(
                    &shared,
                    &mut swarm,
                    mv_topic.as_ref(),
                    reobs_topic.as_ref(),
                    set_update_topic.as_ref(),
                    &mut pending_votes,
                    MAX_PENDING_PER_HEIGHT,
                    &mut ping_failures,
                    &mut dial_failures,
                    &boot_peers,
                    &mut peers_by_attestor,
                    &mut reobs_admission,
                    &peer_registry,
                    event,
                ).await;
                // Recompute the mesh hint after *every* event rather than inside selected event
                // handlers: the mesh can change with no dedicated event (heartbeat GRAFT/PRUNE),
                // so any event-driven update goes stale. This is a cheap in-memory lookup.
                can_broadcast = swarm
                    .behaviour()
                    .gossipsub
                    .mesh_peers(&topic.hash())
                    .next()
                    .is_some();
                can_broadcast_mv = mv_topic.as_ref().is_some_and(|t| {
                    swarm
                        .behaviour()
                        .gossipsub
                        .mesh_peers(&t.hash())
                        .next()
                        .is_some()
                });
                // A topic's mesh just (re)formed — flush that topic's queued votes immediately
                // rather than waiting for the next retry tick. Each topic on its own edge: the
                // meshes are independent, so one forming says nothing about the other.
                if !could_broadcast && can_broadcast && !outbox.is_empty() {
                    let solo = shared.bls_store.is_sole_member(shared.attestor_id.account_id());
                    republish_outbox(&mut outbox, |vote| try_publish(&mut swarm, &topic, vote, solo));
                }
                if !could_broadcast_mv && can_broadcast_mv {
                    if let Some(mv_topic) = &mv_topic {
                        flush_mv_retry_queue(&mut swarm, mv_topic, &mut mv_retry_queue, shared.message_votes.as_ref().expect("message topic has state"));
                    }
                }
            }
        }
    }
}

/// Startup waiting and admission must apply the same policy to each finalized registry snapshot.
pub(crate) fn can_bootstrap_seed(
    local_peer: libp2p::PeerId,
    has_boot_peers: bool,
    reserved_peers: &[libp2p::PeerId],
    registry: &crate::peer_registry::Registry,
) -> bool {
    has_boot_peers
        || registry.has_bootstrap_peers(local_peer)
        || (!registry.registration_required
            && reserved_peers.iter().any(|peer| *peer != local_peer))
}

/// Configure admission independently of discovery: a seed needs trusted peer IDs, but does not
/// need their addresses or an outbound boot route. Configured IDs are authenticated by libp2p's
/// transport handshake; merely claiming an attestor identity in a vote cannot obtain a slot.
fn configure_bootstrap_admission(
    admission: &mut admission::Behaviour,
    boot_peers: &std::collections::HashSet<libp2p::PeerId>,
    reserved_peers: &[libp2p::PeerId],
    local_peer: libp2p::PeerId,
    enable_mdns: bool,
    has_public_addr: bool,
    registry: &crate::peer_registry::Registry,
) -> anyhow::Result<()> {
    let has_boot_peers = boot_peers.iter().any(|peer| *peer != local_peer);

    if has_public_addr && !can_bootstrap_seed(local_peer, has_boot_peers, reserved_peers, registry)
    {
        if registry.registration_required {
            anyhow::bail!(
                "a public seed without usable boot nodes must wait for finalized committee \
                 networking keys; peer registration is required, so --reserved-peers / \
                 ATTESTOR_RESERVED_PEERS / p2p.reserved_peers are ignored"
            );
        }
        anyhow::bail!(
            "a public seed without usable boot nodes requires trusted committee peer IDs in \
             --reserved-peers / ATTESTOR_RESERVED_PEERS / p2p.reserved_peers; otherwise \
             untrusted inbound connections can prevent committee peers from connecting \
             before their first verified vote (the local peer ID does not count)"
        );
    }

    // Reserved peer IDs are not discovery addresses. Without mDNS, a non-seed still needs an
    // outbound bootstrap route; public seeds can instead be dialed by their reserved peers.
    if !has_boot_peers && !enable_mdns && !has_public_addr {
        anyhow::bail!(
            "no usable boot nodes (list empty or every multiaddr missing its /p2p/<peer-id> \
             suffix or referring to this node), mdns is disabled, and no public_addr is set — \
             this node can neither discover a peer nor be discovered as a seed"
        );
    }

    for peer in boot_peers {
        if *peer != local_peer {
            admission.pin(*peer);
        }
    }
    let reserved_peers = if registry.registration_required {
        &[][..]
    } else {
        reserved_peers
    };
    admission.set_configured_peers(
        reserved_peers
            .iter()
            .copied()
            .filter(|peer| *peer != local_peer),
    );
    Ok(())
}

fn ensure_local_peer_key(
    registry: &crate::peer_registry::Registry,
    attestor: &AttestorId,
    local: libp2p::PeerId,
) -> anyhow::Result<()> {
    if registry.peer(attestor).is_some_and(|peer| peer != local) {
        anyhow::bail!("this instance's finalized networking key was rotated; restart with the matching p2p secret");
    }
    Ok(())
}

/// Advance freshness without rebuilding identical reservations. An eviction since the last
/// applied view requires reconciliation even if a removal and reactivation coalesced back to
/// the same membership: that newer state must clear the waiting member's deny binding.
fn update_peer_registry(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    bindings: &mut Vec<(AttestorId, libp2p::PeerId)>,
    current: &crate::peer_registry::Registry,
    next: &crate::peer_registry::Registry,
    fallback: &[libp2p::PeerId],
    minimum_registry_number: &mut u64,
) -> bool {
    if next.finalized_number < *minimum_registry_number {
        return false;
    }
    if !current.same_reservations(next) || *minimum_registry_number > current.finalized_number {
        apply_peer_registry(swarm, bindings, next, fallback, minimum_registry_number)
    } else {
        *minimum_registry_number = next.finalized_number;
        true
    }
}

/// Rebuild dynamic reservations from one finalized view. Gossip may fill missing legacy
/// bindings during rollout, but cannot overwrite a registered networking identity.
fn apply_peer_registry(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    bindings: &mut Vec<(AttestorId, libp2p::PeerId)>,
    registry: &crate::peer_registry::Registry,
    fallback: &[libp2p::PeerId],
    minimum_registry_number: &mut u64,
) -> bool {
    // A removal event may arrive before the independently refreshed registry. Never undo its
    // eviction with a snapshot from before that finalized block (or regress an accepted view).
    if registry.finalized_number < *minimum_registry_number {
        return false;
    }
    *minimum_registry_number = registry.finalized_number;
    let local = *swarm.local_peer_id();
    let previously_reserved = swarm.behaviour().admission.reserved_peers();
    // Waiting identities may connect before election. Do not feed them into the inactive-peer
    // deny gate until they join the voting committee or produce a verified, eligible vote.
    bindings.retain(|(id, peer)| {
        !registry.waiting_attestors.iter().any(|(waiting, key)| {
            waiting == id || key.is_some_and(|waiting_peer| waiting_peer == *peer)
        })
    });
    for (attestor, peer) in &registry.attestors {
        if let Some(peer) = peer {
            note_attestor_peer(bindings, attestor.clone(), *peer);
        }
    }
    // A deactivation revokes the manual fallback for its peer. This snapshot is at or after
    // every accepted deactivation, so a fallback peer tied only to attestors outside it must
    // stay revoked rather than regain a reservation on every refresh.
    let fallback: Vec<libp2p::PeerId> = fallback
        .iter()
        .copied()
        .filter(|peer| {
            !registry.registration_required
                && *peer != local
                && !deny_decision(
                    bindings
                        .iter()
                        .filter(|(_, bound)| bound == peer)
                        .map(|(id, _)| id),
                    |id| registry.is_member(id),
                )
        })
        .collect();
    let admission = &mut swarm.behaviour_mut().admission;
    admission.set_committee_peers(
        registry.registered_peers().filter(|peer| *peer != local),
        registry.registration_required,
    );
    admission.set_configured_peers(fallback);
    admission.set_learned_peers(
        bindings
            .iter()
            .filter(|(id, _)| registry.is_legacy_member(id))
            .map(|(_, peer)| *peer),
    );
    for peer in previously_reserved {
        if !swarm.behaviour().admission.is_reserved(&peer) {
            swarm.behaviour_mut().kad.remove_peer(&peer);
            let _ = swarm.disconnect_peer_id(peer);
        }
    }
    // Retain a bounded history for the existing chilled-peer deny gate, plus every current
    // committee binding. Otherwise repeated elections would grow this vector forever.
    let mut retired = 0;
    let mut kept = Vec::with_capacity(bindings.len());
    for binding in bindings.drain(..).rev() {
        let active = registry.attestors.iter().any(|(id, _)| *id == binding.0);
        if active || retired < common::constants::MAX_ATTESTORS {
            retired += usize::from(!active);
            kept.push(binding);
        }
    }
    kept.reverse();
    *bindings = kept;
    true
}

/// Pending buffer for votes that arrived before local production reached their height:
/// `height -> (attestor -> vote)`. See the construction site in [`run`] for the admission rules.
type PendingVotes = std::collections::HashMap<
    attestor_primitives::Height,
    std::collections::HashMap<AttestorId, Vote>,
>;

/// Remove buffered heights at or below `produced_through`. Votes whose local data exists are
/// returned for verification; the rest are stale/off-schedule and are dropped. Taking all ready
/// heights, rather than only `produced_through`, makes the pending buffer robust to `watch`
/// coalescing during catch-up bursts.
fn drain_pending_votes(
    pending_votes: &mut PendingVotes,
    produced_through: attestor_primitives::Height,
    mut has_local_data: impl FnMut(attestor_primitives::Height) -> bool,
) -> Vec<Vote> {
    let heights = pending_votes
        .keys()
        .copied()
        .filter(|height| *height <= produced_through)
        .collect::<Vec<_>>();
    let mut ready = Vec::new();

    for height in heights {
        let Some(queued) = pending_votes.remove(&height) else {
            continue;
        };
        if has_local_data(height) {
            ready.extend(queued.into_values());
        }
    }

    ready
}

// Reobservation-request admission control (audit P1-4). Reobservation is *unauthenticated* pull
// traffic; without a cap a peer rotating unique message-ids/heights could make every attestor
// re-fetch + re-propagate endlessly. Token-bucket rate-limit BOTH per relaying peer and globally
// *before* forwarding/propagating, so an over-limit flood is dropped (Ignore) rather than amplified
// across the mesh. Generous, since a legitimate relayer only asks when a message stalls.
const REOBS_GLOBAL_CAPACITY: f64 = 20.0;
const REOBS_GLOBAL_REFILL_PER_SEC: f64 = 5.0;
const REOBS_PER_PEER_CAPACITY: f64 = 5.0;
const REOBS_PER_PEER_REFILL_PER_SEC: f64 = 1.0;
const REOBS_MAX_TRACKED_PEERS: usize = 1024;
/// An exact request tuple seen within this window is a replay: dropped without charging any
/// admission budget (CSUB-2138). A genuine retry inside it would only hit the worker's cooldown.
const REOBS_DEDUP_WINDOW: std::time::Duration = REOBS_FAILURE_INTERVAL;
const REOBS_MAX_RECENT: usize = 1024;

/// Monotonic-clock token bucket. Not thread-safe; the swarm loop owns it single-threaded.
#[derive(Clone, Copy)]
struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(capacity: f64, refill_per_sec: f64, now: Instant) -> Self {
        Self {
            tokens: capacity,
            capacity,
            refill_per_sec,
            last: now,
        }
    }

    /// Accrue tokens for the time elapsed since the last touch (capped at capacity). Callers
    /// `refill` + [`has_token`](Self::has_token) several buckets and only [`take`](Self::take) once
    /// they know all will succeed — so a denial doesn't consume a token from any bucket.
    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
    }

    fn has_token(&self) -> bool {
        self.tokens >= 1.0
    }

    /// Consume one token. Caller must have checked [`has_token`](Self::has_token) after a
    /// [`refill`](Self::refill).
    fn take(&mut self) {
        self.tokens -= 1.0;
    }
}

/// Global + per-peer reobservation admission. `admit` consumes a token from BOTH the per-peer and
/// global buckets, but only when both have one — a denial charges neither. (A per-peer bucket keeps
/// one peer from draining the global allowance; charging both atomically stops a global-pressure
/// denial from burning an honest peer's local token and locking it out after capacity returns.)
struct ReobsAdmission {
    global: TokenBucket,
    per_peer: std::collections::HashMap<libp2p::PeerId, TokenBucket>,
    order: std::collections::VecDeque<libp2p::PeerId>,
    /// Recently admitted request tuples. Replaying one is free to drop, so a flood of identical
    /// forged requests (even from many peers) costs the shared global budget a single token.
    recent: CooldownMap<ReobsKey>,
}

impl ReobsAdmission {
    fn new(now: Instant) -> Self {
        Self {
            global: TokenBucket::new(REOBS_GLOBAL_CAPACITY, REOBS_GLOBAL_REFILL_PER_SEC, now),
            per_peer: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
            recent: CooldownMap::new(REOBS_MAX_RECENT),
        }
    }

    /// Whether `key` from `source` may be tried at `now`: not a replay, and both the per-peer and
    /// global buckets hold a token. Charges nothing: a request the queue then turns away must cost
    /// no budget, or identical rejected copies would drain the shared bucket. Call [`commit`] once
    /// the request is actually queued.
    ///
    /// [`commit`]: Self::commit
    fn check(&mut self, key: &ReobsKey, source: libp2p::PeerId, now: Instant) -> bool {
        // A replay is dropped before it can touch the per-peer or global bucket.
        if !self.recent.allow(key, now) {
            return false;
        }
        if !self.per_peer.contains_key(&source) {
            while self.per_peer.len() >= REOBS_MAX_TRACKED_PEERS {
                let Some(old) = self.order.pop_front() else {
                    break;
                };
                self.per_peer.remove(&old);
            }
            self.per_peer.insert(
                source,
                TokenBucket::new(REOBS_PER_PEER_CAPACITY, REOBS_PER_PEER_REFILL_PER_SEC, now),
            );
            self.order.push_back(source);
        }
        // unwrap: just ensured the entry exists. Refill both so the check sees current balances.
        let peer = self.per_peer.get_mut(&source).unwrap();
        peer.refill(now);
        self.global.refill(now);
        peer.has_token() && self.global.has_token()
    }

    /// The request is now held in the queue: charge the per-peer and global buckets and remember
    /// `key`. Only the buckets are charged here, never on a rejection, so a request the queue turned
    /// away costs nothing and cannot block the same (possibly genuine) request arriving via another
    /// neighbour.
    fn commit(&mut self, key: &ReobsKey, source: libp2p::PeerId, now: Instant) {
        if let Some(peer) = self.per_peer.get_mut(&source) {
            peer.take();
        }
        self.global.take();
        self.mark_seen(key, now);
    }

    /// Remember `key` as seen, so identical copies inside [`REOBS_DEDUP_WINDOW`] are dropped for
    /// free.
    fn mark_seen(&mut self, key: &ReobsKey, now: Instant) {
        self.recent.record(*key, now, REOBS_DEDUP_WINDOW);
    }

    /// Check and, if allowed, commit in one step: the semantics the admission tests were written
    /// against, for a request the queue always accepts.
    #[cfg(test)]
    fn admit(&mut self, key: &ReobsKey, source: libp2p::PeerId, now: Instant) -> bool {
        if !self.check(key, source, now) {
            return false;
        }
        self.commit(key, source, now);
        true
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_swarm(
    shared: &Arc<Shared>,
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    mv_topic: Option<&libp2p::gossipsub::IdentTopic>,
    reobs_topic: Option<&libp2p::gossipsub::IdentTopic>,
    set_update_topic: Option<&libp2p::gossipsub::IdentTopic>,
    pending_votes: &mut PendingVotes,
    max_pending_per_height: usize,
    ping_failures: &mut std::collections::HashMap<libp2p::swarm::ConnectionId, u32>,
    dial_failures: &mut std::collections::HashMap<libp2p::PeerId, u32>,
    boot_peers: &std::collections::HashSet<libp2p::PeerId>,
    peers_by_attestor: &mut Vec<(AttestorId, libp2p::PeerId)>,
    reobs_admission: &mut ReobsAdmission,
    peer_registry: &crate::peer_registry::Registry,
    event: libp2p::swarm::SwarmEvent<behavior::P2PBehaviorEvent>,
) {
    use behavior::P2PBehaviorEvent;
    use libp2p::swarm::SwarmEvent;

    match event {
        SwarmEvent::Behaviour(P2PBehaviorEvent::Identify(libp2p::identify::Event::Received {
            peer_id,
            info: libp2p::identify::Info { listen_addrs, .. },
            connection_id,
        })) => {
            tracing::debug!(%peer_id, %connection_id, "🛰️ discovered peer");
            // Deny-list gate: don't re-populate the routing table for a peer belonging to a
            // deactivated attestor. Without this, discovery would silently undo the eviction the
            // moment a still-running chilled node re-announces itself.
            if is_peer_denied(shared, peers_by_attestor, &peer_id) {
                tracing::info!(%peer_id, "🚫 ignoring discovery for denied (chilled/kicked) peer");
                evict_peer(swarm, peer_id);
                return;
            }
            for a in listen_addrs {
                swarm.behaviour_mut().kad.add_address(&peer_id, a);
            }
        }
        SwarmEvent::Behaviour(P2PBehaviorEvent::Mdns(libp2p::mdns::Event::Discovered(peers))) => {
            for (peer_id, address) in peers {
                if is_peer_denied(shared, peers_by_attestor, &peer_id) {
                    tracing::info!(%peer_id, "🚫 ignoring mdns discovery for denied peer");
                    evict_peer(swarm, peer_id);
                    continue;
                }
                tracing::info!(%peer_id, %address, "🛰️ local mdns peer");
                swarm.behaviour_mut().kad.add_address(&peer_id, address);
            }
        }
        SwarmEvent::Behaviour(P2PBehaviorEvent::Kad(libp2p::kad::Event::RoutingUpdated {
            peer,
            is_new_peer,
            addresses,
            old_peer,
            ..
        })) => {
            if is_new_peer {
                tracing::info!(peer_id = %peer, addrs = addresses.len(), "📋 new routing peer");
                shared.metrics.note_routing_peer_added();
            }
            if let Some(evicted) = old_peer {
                tracing::info!(peer_id = %evicted, "📋 evicted routing peer");
                shared.metrics.note_routing_peer_evicted();
            }
        }
        SwarmEvent::Behaviour(P2PBehaviorEvent::Ping(libp2p::ping::Event {
            peer,
            connection,
            result,
        })) => match result {
            Ok(rtt) => {
                ping_failures.remove(&connection);
                tracing::debug!(peer_id = %peer, %connection, rtt_ms = rtt.as_millis(), "🔔 pong")
            }
            Err(err) => {
                let failures = *ping_failures
                    .entry(connection)
                    .and_modify(|count| *count += 1)
                    .or_insert(1);
                tracing::error!(peer_id = %peer, %connection, failures, %err, "🔕 ping failed");
                if failures >= MAX_PING_FAILURES {
                    tracing::warn!(
                        peer_id = %peer,
                        %connection,
                        failures,
                        "✂️  closing connection after repeated ping failures",
                    );
                    ping_failures.remove(&connection);
                    swarm.close_connection(connection);
                }
            }
        },
        SwarmEvent::Behaviour(P2PBehaviorEvent::Gossipsub(libp2p::gossipsub::Event::Message {
            propagation_source,
            message_id,
            message,
        })) => {
            shared.metrics.increase_gossipsub_message_count();

            // Route by topic: message votes / reobservation requests (write-ability) vs block
            // attestations. All ride this one swarm; the write-ability topics are `Some` only when
            // message attestation is enabled.
            let is_message_vote = mv_topic.is_some_and(|t| message.topic == t.hash());
            let is_reobs = reobs_topic.is_some_and(|t| message.topic == t.hash());
            let is_set_update = set_update_topic.is_some_and(|t| message.topic == t.hash());
            let decision = if is_message_vote {
                handle_message_vote(shared, &message.data)
            } else if is_reobs {
                handle_reobservation_request(
                    shared,
                    &message.data,
                    reobs_admission,
                    propagation_source,
                )
            } else if is_set_update {
                // We subscribe to the attestor-set-update topic only to publish our proposer's own
                // votes and keep the mesh propagating them — attestors do NOT aggregate set-update
                // votes (the relayer does). We can't decode to tell a legitimate vote from garbage,
                // so we must not `Reject`: that would penalize honest relayers of real set-update
                // votes (a prior bugbot fix). But `Accept`ing *everything* lets a peer amplify spam
                // at no reputation cost. Resolve both by bounding size: a SetUpdateVote is a small
                // SCALE blob (a bounded attestor set + one signature + chain id + nonce), so anything
                // past `MAX_SET_UPDATE_FRAME_BYTES` is not a real vote — `Ignore` it (no propagation,
                // no peer penalty). Legitimately-sized frames still `Accept` and relay (bugbot).
                if message.data.len() > MAX_SET_UPDATE_FRAME_BYTES {
                    libp2p::gossipsub::MessageAcceptance::Ignore
                } else {
                    libp2p::gossipsub::MessageAcceptance::Accept
                }
            } else {
                let (acceptance, learned) = handle_vote_msg(
                    shared,
                    pending_votes,
                    max_pending_per_height,
                    peers_by_attestor,
                    message.source,
                    &message.data,
                )
                .await;

                // During backfill, legacy bindings use the original signed publisher rather than
                // the relaying neighbour. Registered controllers always use the finalized mapping.
                if let (Some(attestor), Some(source)) = (learned, message.source) {
                    // A valid BLS vote can be republished by another transport identity. Once
                    // registered, only the finalized on-chain mapping grants reserved capacity.
                    if let Some(old) = note_verified_vote_peer(
                        &mut swarm.behaviour_mut().admission,
                        peers_by_attestor,
                        peer_registry,
                        attestor,
                        source,
                    ) {
                        evict_replaced_peer(swarm, peers_by_attestor, peer_registry, old);
                    }
                }

                match acceptance {
                    Acceptance::Accept => libp2p::gossipsub::MessageAcceptance::Accept,
                    Acceptance::Ignore => libp2p::gossipsub::MessageAcceptance::Ignore,
                    Acceptance::Reject => {
                        shared.metrics.increase_invalid_gossipsub_count();
                        libp2p::gossipsub::MessageAcceptance::Reject
                    }
                }
            };
            if let Err(err) = swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&message_id, &propagation_source, decision)
            {
                tracing::warn!(%message_id, %err, "gossipsub validation-result report failed");
            }
        }
        // Subscription and gossipsub-support events. These carry no vote, but the absence of
        // them is the signature of a stalled set. Topics are per-chain, so a peer from a
        // *different* attestor set never subscribes to your topic and that is normal: only a
        // missing `Subscribed` from a peer of your own set is a signal. Such a peer has
        // negotiated a TCP/QUIC connection without a working gossipsub substream, so it never
        // enters the topic's `topic_peers` and never receives or forwards a vote. Publishing can
        // still return `Ok` while other topic peers exist, so this is invisible from the message
        // counters alone; if the stalled peer is the only one on the topic, `publish` returns
        // `InsufficientPeers` instead.
        SwarmEvent::Behaviour(P2PBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Subscribed { peer_id, topic },
        )) => {
            tracing::info!(%peer_id, %topic, "📬 gossipsub subscribed");
        }
        SwarmEvent::Behaviour(P2PBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Unsubscribed { peer_id, topic },
        )) => {
            tracing::info!(%peer_id, %topic, "📭 gossipsub unsubscribed");
        }
        // Fires when the connection is up but the peer never negotiated the gossipsub protocol
        // on it, which is the dead-substream case above: connection-up counts, subscription
        // exchange never happens, no vote is ever delivered in either direction. Unlike a missing
        // `Subscribed`, this one is unambiguous, since it says nothing about which topic is
        // involved.
        SwarmEvent::Behaviour(P2PBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::GossipsubNotSupported { peer_id },
        )) => {
            shared.metrics.increase_gossipsub_unsupported_peer_count();
            tracing::warn!(%peer_id, "🚫 peer does not support gossipsub");
        }
        SwarmEvent::ConnectionClosed {
            connection_id,
            num_established,
            ..
        } => {
            // Only decrement the per-peer gauge when this was the *last* connection to the
            // remote peer (`num_established` is the post-close count). The swarm allows multiple
            // simultaneous transports (TCP + QUIC) and up to a per-peer connection limit, so
            // a single peer can produce several ConnectionEstablished / ConnectionClosed pairs.
            // Counting all of them would inflate the gauge and let it diverge from “distinct
            // peers I'm actually talking to right now”.
            if num_established == 0 {
                shared.metrics.note_peer_disconnected();
                shared.health.note_peer_disconnected();
            }
            ping_failures.remove(&connection_id);
            // The mesh hint (`can_broadcast`) is recomputed in the run loop after every event —
            // no per-event update needed here.
        }
        SwarmEvent::NewListenAddr {
            listener_id,
            address,
        } => {
            if let Ok(address) = address.with_p2p(*swarm.local_peer_id()) {
                tracing::info!(%listener_id, %address, "🔍 new listen addr");
            }
        }
        SwarmEvent::ConnectionEstablished {
            peer_id,
            connection_id,
            num_established,
            ..
        } => {
            tracing::info!(%peer_id, %connection_id, "🔗 connection up");
            // Track *distinct* connected peers — only bump the gauge when this is the first
            // established connection to the remote peer (`num_established == 1` includes this
            // one). The swarm enables both TCP and QUIC and allows multiple simultaneous
            // connections per peer, so a single peer can fire ConnectionEstablished several
            // times; counting each one would have the gauge measure connections, not peers.
            // Separate from the Kademlia routing-table gauge (`note_routing_peer_added`),
            // which can hold a peer with no live connection.
            if num_established.get() == 1 {
                shared.metrics.note_peer_connected();
                shared.health.note_peer_connected();
            }
            // Any established connection (either direction) proves the peer reachable — restart
            // its dial-failure count from a clean slate.
            dial_failures.remove(&peer_id);
            // Deny-list gate for *incoming* connections: a chilled node that keeps running will
            // dial us. We disconnect *after* the gauge bump above so the paired
            // `ConnectionClosed` decrement keeps the connected-peer count balanced (it's an
            // unsigned gauge — an unbalanced `dec()` would underflow). Its gossip is already
            // rejected via the BLS allow-set; this stops it from holding a connection slot.
            if is_peer_denied(shared, peers_by_attestor, &peer_id) {
                tracing::info!(%peer_id, %connection_id, "🚫 dropping connection from denied peer");
                evict_peer(swarm, peer_id);
            }
        }
        SwarmEvent::OutgoingConnectionError {
            peer_id,
            connection_id,
            error,
        } => {
            tracing::warn!(?peer_id, %connection_id, %error, "⛔ outgoing connection error");
            shared.metrics.increase_connection_failure_count();
            match error {
                // Unambiguously malicious / unrecoverable — drop immediately (v1 logic).
                libp2p::swarm::DialError::WrongPeerId { .. }
                | libp2p::swarm::DialError::Denied { .. } => {
                    if let Some(p) = peer_id {
                        swarm.behaviour_mut().kad.remove_peer(&p);
                        dial_failures.remove(&p);
                    }
                }
                // Transport-level failure (timeout, refused, handshake error) — genuine evidence
                // of unreachability. Count it, and once a peer racks up MAX_DIAL_FAILURES in a
                // row without a single established connection, drop it from the routing table so
                // we stop redialing a node that shut down while still registered on-chain.
                // Deliberately NOT counted: `DialPeerConditionFalse` (dial suppressed, nothing
                // attempted), `NoAddresses` (nothing to evict), `Aborted`/`LocalPeerId` (not
                // reachability evidence).
                libp2p::swarm::DialError::Transport(_) => {
                    if let Some(p) = peer_id {
                        if note_dial_failure(dial_failures, boot_peers, p) {
                            tracing::info!(
                                peer_id = %p,
                                failures = MAX_DIAL_FAILURES,
                                "🧹 evicting unreachable peer after repeated dial failures"
                            );
                            swarm.behaviour_mut().kad.remove_peer(&p);
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

// ------------------------------------* publish + retry *-------------------------------------- //

/// Publish one vote to the gossip topic. Returns `true` when the vote needs no further retry:
/// either it was published, or gossipsub reports it a duplicate (already in the message cache
/// from a previous successful publish). Any other failure returns `false` so the caller can
/// queue the vote for retry.
///
/// `solo` is `true` when this attestor is the only elected member of its set. It then reaches
/// quorum on its own vote and has no peer to gossip to, so `InsufficientPeers` is the expected
/// answer and is logged at debug instead of warn. Retry behaviour does not change.
fn try_publish(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    vote: &Vote,
    solo: bool,
) -> bool {
    match swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.hash(), vote.encode())
    {
        Ok(_) => {
            tracing::info!(
                digest = ?vote.digest,
                height = vote.height,
                attestor = %vote.attestor,
                "✉️ gossiped vote",
            );
            true
        }
        Err(libp2p::gossipsub::PublishError::Duplicate) => true,
        Err(err) if publish_failure_is_expected(&err, solo) => {
            tracing::debug!(
                digest = ?vote.digest,
                height = vote.height,
                %err,
                "✉️ no gossip peers: this attestor is the only elected member, so there is nobody \
                 to broadcast to",
            );
            false
        }
        Err(err) => {
            tracing::warn!(
                digest = ?vote.digest,
                height = vote.height,
                %err,
                "✉️ gossip publish failed — vote stays in the outbox for re-gossip",
            );
            false
        }
    }
}

/// Publish one message vote to the message-votes topic. Returns `true` when the vote needs no
/// further retry: either it was published, or gossipsub reports it a duplicate (already in the
/// message cache from a previous successful publish). Any other failure returns `false` so the
/// caller can queue the vote for retry.
fn try_publish_message_vote(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    vote: &pending::PendingVote,
    state: &crate::tasks::write_ability::MessageVoteState,
) -> bool {
    match swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.hash(), vote.encode_bytes())
    {
        Ok(_) => {
            state.pending.acknowledge(vote.key.clone());
            tracing::info!(chain_key = vote.chain_key, "✉️ gossiped message vote");
            true
        }
        Err(libp2p::gossipsub::PublishError::Duplicate) => {
            state.pending.acknowledge(vote.key.clone());
            true
        }
        Err(err) => {
            tracing::warn!(
                chain_key = vote.chain_key,
                %err,
                "✉️ message-vote publish failed — queueing for retry",
            );
            false
        }
    }
}

/// Publish an attestor-set-update vote. No retry queue — the proposer re-emits while the set stays
/// diverged, so a transient publish failure (typically no mesh peers yet) is recovered next cycle.
fn try_publish_set_update_vote(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    vote: &SetUpdateVote,
) {
    match swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.hash(), vote.encode_bytes())
    {
        Ok(_) => {
            tracing::info!(
                chain_key = vote.chain_key,
                attestors = vote.new_attestors.len(),
                "🗳️ gossiped attestor-set-update vote"
            );
        }
        Err(libp2p::gossipsub::PublishError::Duplicate) => {}
        Err(err) => {
            tracing::warn!(
                chain_key = vote.chain_key,
                %err,
                "🗳️ attestor-set-update publish failed — proposer will re-emit next cycle",
            );
        }
    }
}

/// Append a message vote to its bounded retry queue, dropping the oldest entry on overflow (the
/// one whose aggregation window is closest to expiring anyway).
fn queue_message_vote_for_retry(
    mv_retry_queue: &mut std::collections::VecDeque<pending::PendingVote>,
    vote: pending::PendingVote,
) {
    if mv_retry_queue.len() >= MAX_OUTBOX {
        if let Some(dropped) = mv_retry_queue.pop_front() {
            tracing::warn!(
                chain_key = dropped.chain_key,
                cap = MAX_OUTBOX,
                "📭 message-vote retry cache full — evicted entry remains available through durable replay"
            );
        }
    }
    mv_retry_queue.push_back(vote);
}

/// Republish queued message votes in order (oldest first) until one fails, which usually means
/// the mesh went away again — the remainder stays queued for the next trigger.
fn flush_mv_retry_queue(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    mv_retry_queue: &mut std::collections::VecDeque<pending::PendingVote>,
    state: &crate::tasks::write_ability::MessageVoteState,
) {
    let backlog = mv_retry_queue.len();
    while let Some(vote) = mv_retry_queue.front() {
        if try_publish_message_vote(swarm, topic, vote, state) {
            mv_retry_queue.pop_front();
        } else {
            break;
        }
    }
    if mv_retry_queue.len() < backlog {
        tracing::info!(
            published = backlog - mv_retry_queue.len(),
            remaining = mv_retry_queue.len(),
            "📤 flushed unpublished message-vote backlog"
        );
    }
}

/// A solo attestor's publish fails with `InsufficientPeers` on every vote and every retry tick.
/// That is expected, not a broken mesh; any other error, or the same error in a real set, is not.
fn publish_failure_is_expected(err: &libp2p::gossipsub::PublishError, solo: bool) -> bool {
    solo && matches!(err, libp2p::gossipsub::PublishError::InsufficientPeers)
}

/// Append a vote to the bounded outbox, dropping the oldest entry on overflow (lowest height —
/// the one most likely to finalize without our broadcast).
fn enqueue_unfinalized(outbox: &mut std::collections::VecDeque<Vote>, vote: Vote) {
    if outbox.len() >= MAX_OUTBOX {
        if let Some(dropped) = outbox.pop_front() {
            tracing::warn!(
                digest = ?dropped.digest,
                height = dropped.height,
                cap = MAX_OUTBOX,
                "🗑️ outbox full — dropping oldest unfinalized vote"
            );
        }
    }
    outbox.push_back(vote);
}

/// Re-gossip queued votes in order (oldest first), stopping at the first failure — which usually
/// means the mesh went away again; the rest wait for the next trigger. Votes are NOT removed on
/// publish success: gossipsub Ok is hand-off, not delivery, and the entry only leaves the outbox
/// when its height finalizes on-chain (the `latest_finalized_rx` prune). Takes the publisher as a
/// closure so the resend policy is testable without a swarm.
fn republish_outbox(
    outbox: &mut std::collections::VecDeque<Vote>,
    mut publish: impl FnMut(&Vote) -> bool,
) -> usize {
    let mut published = 0;
    for vote in outbox.iter() {
        if publish(vote) {
            published += 1;
        } else {
            break;
        }
    }
    if published > 0 {
        tracing::debug!(
            published,
            queued = outbox.len(),
            "📤 re-gossiped unfinalized vote backlog"
        );
    }
    published
}

// -------------------------------------* peer deny-list *-------------------------------------- //

/// Opportunistic vote-authorship bindings exist only for unregistered members during rollout.
/// A copied valid vote cannot override or reserve a finalized registered transport identity.
/// Returns the peer that lost its learned reservation because the attestor moved off it.
fn note_verified_vote_peer(
    admission: &mut admission::Behaviour,
    bindings: &mut Vec<(AttestorId, libp2p::PeerId)>,
    registry: &crate::peer_registry::Registry,
    attestor: AttestorId,
    source: libp2p::PeerId,
) -> Option<libp2p::PeerId> {
    if let Some(peer) = registry.peer(&attestor) {
        // A waiting member can be elected before the next registry refresh. A verified vote
        // then installs its canonical binding, so a later deactivation is still evicted.
        note_attestor_peer(bindings, attestor, peer);
        admission.reserve(peer);
        return None;
    }
    if !registry.is_legacy_member(&attestor) {
        return None;
    }
    let mut released = None;
    if let Some(old) = note_attestor_peer(bindings, attestor, source) {
        if !bindings
            .iter()
            .any(|(id, peer)| *peer == old && registry.is_legacy_member(id))
        {
            admission.unreserve(&old);
            released = Some(old);
        }
    }
    admission.reserve(source);
    released
}

/// After a vote moved an attestor off `old`: drop `old` from the routing table and close its
/// connection if it holds no other reservation and only maps to attestors that are no longer
/// members. A peer with no mapping is left alone, as the deny gate never targets those.
fn evict_replaced_peer(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    bindings: &[(AttestorId, libp2p::PeerId)],
    registry: &crate::peer_registry::Registry,
    old: libp2p::PeerId,
) {
    if swarm.behaviour().admission.is_reserved(&old) {
        return;
    }
    let mapped = bindings
        .iter()
        .filter(|(_, peer)| *peer == old)
        .map(|(id, _)| id);
    if deny_decision(mapped, |id| registry.is_member(id)) {
        evict_peer(swarm, old);
    }
}

/// Record one peer per attestor, returning its previous peer ID after a rotation.
fn note_attestor_peer(
    peers_by_attestor: &mut Vec<(AttestorId, libp2p::PeerId)>,
    attestor: AttestorId,
    peer_id: libp2p::PeerId,
) -> Option<libp2p::PeerId> {
    // `position` (shared borrow) then index (mutable borrow) keeps the two borrows disjoint, so
    // the `push` in the not-found case is accepted without relying on Polonius.
    if let Some(pos) = peers_by_attestor.iter().position(|(a, _)| *a == attestor) {
        let existing = &mut peers_by_attestor[pos].1;
        if *existing != peer_id {
            tracing::info!(%attestor, old = %existing, new = %peer_id, "🔁 attestor peer id changed");
            return Some(std::mem::replace(existing, peer_id));
        }
    } else {
        tracing::debug!(%attestor, %peer_id, "🪪 learned attestor peer id");
        peers_by_attestor.push((attestor, peer_id));
    }
    None
}

/// Pure deny decision, split out so it can be unit tested without a live swarm or chain.
///
/// A peer is denied iff we have mapped it to at least one attestor **and none** of those attestors
/// is currently active. A peer we've never associated with an attestor is never denied — we only
/// gate peers we can positively tie to a removed attestor, never arbitrary addresses. Keying on
/// attestor identity (not network address) is deliberate: the same address can host an unrelated,
/// legitimate peer, and a node that re-registers under a new active attestor id sharing one peer id
/// stays allowed because that active mapping short-circuits the decision.
fn deny_decision<'a>(
    mapped_attestors: impl Iterator<Item = &'a AttestorId>,
    is_active: impl Fn(&AttestorId) -> bool,
) -> bool {
    let mut mapped = false;
    for attestor in mapped_attestors {
        mapped = true;
        if is_active(attestor) {
            return false;
        }
    }
    mapped
}

/// [`deny_decision`] wired to live state: "is this attestor active" is answered by `bls_store`,
/// which the production task refreshes on every chill/kick/election, so this needs no separate
/// deny-list state to keep in sync — it always reflects current on-chain status.
fn is_peer_denied(
    shared: &Arc<Shared>,
    peers_by_attestor: &[(AttestorId, libp2p::PeerId)],
    peer_id: &libp2p::PeerId,
) -> bool {
    deny_decision(
        peers_by_attestor
            .iter()
            .filter(|(_, p)| p == peer_id)
            .map(|(a, _)| a),
        |a| shared.bls_store.pubkey(a.account_id()).is_some(),
    )
}

/// Remove a peer from the Kademlia routing table and force-close any live connection to it.
/// Only the vote-learned reservation is released: the deny gate reads `bls_store`, which can
/// trail the finalized registry, so finalized committee capacity is revoked solely by
/// [`handle_peer_deactivated`] and registry refreshes. Takes the [`PeerId`] by value because
/// `disconnect_peer_id` consumes it and `PeerId` is not `Copy`.
///
/// [`PeerId`]: libp2p::PeerId
fn evict_peer(swarm: &mut libp2p::Swarm<behavior::P2PBehavior>, peer_id: libp2p::PeerId) {
    swarm.behaviour_mut().admission.unreserve(&peer_id);
    swarm.behaviour_mut().kad.remove_peer(&peer_id);
    if swarm.disconnect_peer_id(peer_id).is_ok() {
        tracing::info!(%peer_id, "✂️  closed connection to denied peer");
    }
}

/// Records one transport-level dial failure for `peer_id` and reports whether the peer just
/// crossed [`MAX_DIAL_FAILURES`] and should be evicted from the routing table. On eviction the
/// counter is cleared so a later rediscovery of the same peer starts a fresh count instead of
/// being evicted on its first failure. Boot nodes never evict — they are the rendezvous points
/// the mesh reforms through, so their counter isn't even tracked. Split out from the swarm arm
/// so the counting/threshold logic is unit-testable without a live swarm.
fn note_dial_failure(
    dial_failures: &mut std::collections::HashMap<libp2p::PeerId, u32>,
    boot_peers: &std::collections::HashSet<libp2p::PeerId>,
    peer_id: libp2p::PeerId,
) -> bool {
    if boot_peers.contains(&peer_id) {
        return false;
    }
    let count = dial_failures.entry(peer_id).or_insert(0);
    *count += 1;
    if *count >= MAX_DIAL_FAILURES {
        dial_failures.remove(&peer_id);
        return true;
    }
    false
}

/// Handle a production nudge that an attestor was chilled/kicked: evict every peer we've mapped to
/// it that is now denied. We keep the `(attestor, peer)` binding afterwards so the discovery /
/// connection gates keep rejecting the peer while it stays chilled; once it reactivates,
/// `bls_store` reports it active again, [`is_peer_denied`] returns false, and ordinary discovery
/// lets it back in — no explicit "re-add" path needed. A peer id shared with a still-active
/// attestor is left untouched.
fn handle_peer_deactivated(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    peers_by_attestor: &mut Vec<(AttestorId, libp2p::PeerId)>,
    registry: &crate::peer_registry::Registry,
    minimum_registry_number: &mut u64,
    event: &crate::shared::PeerDeactivated,
    is_active: impl Fn(&AttestorId) -> bool,
) {
    // A registry at/after this event already contains its post-block state. Replaying an old
    // removal must not revoke a subsequent reactivation (including one in the same block), but
    // a peer that redialed before `bls_store` caught up is still evicted when neither view
    // keeps it.
    let covered = event.finalized_number <= registry.finalized_number;
    let attestor = &event.attestor;
    if !covered {
        *minimum_registry_number = (*minimum_registry_number).max(event.finalized_number);
        // Waiting members have a reservation before any vote, but deliberately do not enter
        // the inactive-voter deny map. An explicit deactivation now supplies that binding.
        if let Some(peer) = registry.peer(attestor) {
            note_attestor_peer(peers_by_attestor, attestor.clone(), peer);
        }
    }
    let peers: Vec<libp2p::PeerId> = peers_by_attestor
        .iter()
        .filter(|(a, _)| a == attestor)
        .map(|(_, p)| *p)
        .collect();

    if peers.is_empty() {
        tracing::debug!(%attestor, "🚫 deactivated attestor has no known peer — nothing to evict");
        return;
    }

    for peer_id in peers {
        if !deny_decision(
            peers_by_attestor
                .iter()
                .filter(|(_, peer)| *peer == peer_id)
                .map(|(id, _)| id),
            &is_active,
        ) {
            tracing::debug!(%attestor, %peer_id, "↩️  peer still maps to an active attestor — keeping");
            continue;
        }
        if covered {
            if registry.registered_peers().any(|peer| peer == peer_id) {
                continue;
            }
        } else {
            swarm.behaviour_mut().admission.revoke(&peer_id);
        }
        tracing::info!(%attestor, %peer_id, "🚫 evicting peer for deactivated attestor");
        evict_peer(swarm, peer_id);
    }
}

enum Acceptance {
    Accept,
    Ignore,
    Reject,
}

/// Validate + count an incoming message vote (write-ability), mapping the result to a gossipsub
/// acceptance. Delegates the real work to [`ingest::validate_and_count`]; we only translate the
/// decision and surface a reached-threshold milestone.
fn handle_message_vote(shared: &Arc<Shared>, bytes: &[u8]) -> libp2p::gossipsub::MessageAcceptance {
    use libp2p::gossipsub::MessageAcceptance;
    let Some(state) = &shared.message_votes else {
        // Topic isn't subscribed when disabled, so this is unreachable in practice.
        return MessageAcceptance::Ignore;
    };
    match ingest::validate_and_count(state, shared.chain_key, bytes) {
        ingest::Acceptance::Accept {
            reached_threshold,
            message_id,
        } => {
            shared.metrics.note_message_vote();
            if reached_threshold {
                ingest::note_threshold(shared.chain_key, &message_id);
            }
            MessageAcceptance::Accept
        }
        ingest::Acceptance::Ignore => MessageAcceptance::Ignore,
        ingest::Acceptance::Reject => {
            shared.metrics.increase_invalid_gossipsub_count();
            MessageAcceptance::Reject
        }
    }
}

/// Decode a reobservation request and hand it to the write-ability task to verify + re-sign. The
/// swarm loop must stay responsive, so we only decode + queue here (no RPC / signing). We Accept a
/// well-formed request only once it is queued, so it keeps propagating to other attestors — each
/// re-verifies it independently. A request we could not queue (duplicate, peer over its share, queue
/// full) is `Ignore`d: not propagated, so a flood is not amplified across the mesh.
///
/// Because this `Accept` precedes verification and [`ReobservationRequest`] is unsigned, the
/// reobservation topic is registered in `penalty_only_topics` (see the topic split in [`run`]) so
/// the Accept earns no P2 credit. Keep it there: crediting it would make well-formed requests naming
/// arbitrary message ids a free source of peer score.
fn handle_reobservation_request(
    shared: &Arc<Shared>,
    bytes: &[u8],
    admission: &mut ReobsAdmission,
    source: libp2p::PeerId,
) -> libp2p::gossipsub::MessageAcceptance {
    use libp2p::gossipsub::MessageAcceptance;
    let Some(state) = &shared.message_votes else {
        return MessageAcceptance::Ignore;
    };
    // Decode first (four fixed fields, no I/O) so replays and wrong-chain frames can be dropped
    // without charging admission budget — a replayed forged tuple must not drain the global bucket.
    let request = match ReobservationRequest::decode_bytes(bytes) {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(%err, "⛔ failed to decode reobservation request");
            shared.metrics.increase_invalid_gossipsub_count();
            return MessageAcceptance::Reject;
        }
    };
    if request.chain_key != shared.chain_key {
        return MessageAcceptance::Ignore;
    }
    intake_reobservation(
        admission,
        &state.reobs_queue,
        source,
        request,
        Instant::now(),
    )
}

/// Admission then queueing for one decoded request. Rate-limit before doing (or propagating) any
/// work: an over-limit or replayed reobservation is dropped with `Ignore` so it is not amplified
/// across the mesh (audit P1-4). Budget and the replay filter are only charged once the request is
/// held in the queue, so a copy the queue turned away costs nothing and cannot shadow the genuine
/// one or drain the shared bucket.
fn intake_reobservation(
    admission: &mut ReobsAdmission,
    queue: &ReobsQueue,
    source: libp2p::PeerId,
    request: ReobservationRequest,
    now: Instant,
) -> libp2p::gossipsub::MessageAcceptance {
    use libp2p::gossipsub::MessageAcceptance;
    let key = ReobsKey::from(&request);
    if !admission.check(&key, source, now) {
        tracing::debug!(%source, "🚦 reobservation rate limit exceeded or replayed — dropping, not propagating");
        return MessageAcceptance::Ignore;
    }
    let push = queue.push(source, request);
    match push {
        QueuePush::Queued => admission.commit(&key, source, now),
        // Already queued by an earlier copy: nothing new was admitted, so no charge.
        QueuePush::Duplicate => admission.mark_seen(&key, now),
        QueuePush::Full => {
            tracing::warn!("reobservation queue full — dropping and not propagating");
        }
        QueuePush::PeerFull => {}
    }
    reobs_push_acceptance(push)
}

/// Gossipsub verdict for a queue outcome: only a request we actually queued keeps propagating.
fn reobs_push_acceptance(push: QueuePush) -> libp2p::gossipsub::MessageAcceptance {
    use libp2p::gossipsub::MessageAcceptance;
    match push {
        QueuePush::Queued => MessageAcceptance::Accept,
        QueuePush::Duplicate | QueuePush::PeerFull | QueuePush::Full => MessageAcceptance::Ignore,
    }
}

async fn handle_vote_msg(
    shared: &Arc<Shared>,
    pending_votes: &mut PendingVotes,
    max_pending_per_height: usize,
    peers_by_attestor: &[(AttestorId, libp2p::PeerId)],
    source: Option<libp2p::PeerId>,
    bytes: &[u8],
) -> (Acceptance, Option<AttestorId>) {
    // `decode_all` enforces canonical SCALE consumption — payloads with trailing bytes are
    // malformed gossip and rejected outright rather than silently accepted.
    let Ok(vote) = Vote::decode_all(&mut &bytes[..]) else {
        tracing::warn!("⛔ failed to decode vote");
        return (Acceptance::Reject, None);
    };

    let local = shared.proof_cache.local_data(vote.height);
    let pubkey = shared.bls_store.pubkey(vote.attestor.account_id());
    let result = verify_vote(&vote, shared.chain_key, local.as_ref(), pubkey.as_ref());

    // Only trust (and later act on) the peer → attestor binding once the vote has BLS-verified as
    // genuinely this attestor's; `Accept` is the only such outcome.
    let learned = matches!(result, VerifyResult::Accept).then(|| vote.attestor.clone());

    let acceptance = match result {
        VerifyResult::Accept => match shared.pool_send.send(vote.clone()) {
            Some(Ok(())) => Acceptance::Accept,
            Some(Err(err)) => {
                err.log_error(vote.digest);
                use attestor_pool::Error as PoolError;
                match err {
                    // Out of catch-up window or below finalization — ignore (not malicious).
                    PoolError::InvalidHeight(..) => Acceptance::Ignore,
                    // Equivocation — ignore (don't help propagate).
                    PoolError::Equivocation(..) => {
                        shared.metrics.increase_equivocation_count();
                        Acceptance::Ignore
                    }
                    // Ignore, NOT Reject: both verdicts depend on *our local view* of chain
                    // state, not on anything provably wrong with the message — and the vote
                    // already BLS-verified, so the sender is a real attestor. `Unauthorized`
                    // can be our allow-set lagging an election; `KnownInvalid` a fork we
                    // tombstoned that peers legitimately voted on. Reject feeds gossipsub's
                    // P4 penalty (weight −10, squared, ~10 min decay): three of these inside
                    // the window graylists an honest committee peer — quorum-relevant at
                    // small committee sizes, and a negative retained score also blocks the
                    // peer's mesh re-entry after a reconnect. Reject is reserved for
                    // provable invalidity (decode failure, bad signature, wrong chain,
                    // spoofed identity claim).
                    PoolError::Unauthorized(..) | PoolError::KnownInvalid(..) => Acceptance::Ignore,
                }
            }
            None => Acceptance::Ignore,
        },
        VerifyResult::NoLocal => {
            // We haven't produced at this height yet. Before buffering, gate on cheap checks
            // so a peer can't grow this map without bound:
            //   * membership — we can't BLS-verify without local data, but we CAN check the sender
            //     is in the active attestor set. `verify_vote` returns `NoLocal` *before* its
            //     membership check, so do it here; an unknown sender is rejected outright rather
            //     than buffered.
            //   * producible height — the height must be one we could actually produce locally
            //     (on the attestation schedule and within the catch-up window). Off-schedule or
            //     far-future heights never gain local data, so buffering them is pure memory-
            //     attack surface; drop them (Ignore) instead.
            //   * sender identity — the vote's BLS signature can't be checked yet, so a peer
            //     could otherwise occupy pending slots by *claiming* an active attestor's
            //     identity. If we've already learned the attestor's libp2p peer id (from an
            //     earlier BLS-verified vote), require the gossipsub-authenticated original
            //     publisher (`message.source`, which survives relaying — NOT the relaying
            //     `propagation_source`) to match it; a mismatch is a spoofed claim and is
            //     rejected. Without a binding yet we can't authenticate, so we admit
            //     first-come per `(height, attestor)` — the per-attestor keying still bounds
            //     the damage to one contested slot per claimed identity.
            if pubkey.is_none() {
                // Ignore, NOT Reject: "unknown" means *our* bls_store has no pubkey for the
                // claimed attestor — which is just as often our own staleness (an election we
                // haven't processed, a missing on-chain BLS key registration) as a bogus
                // identity. Rejecting would P4-penalize whoever relayed it (see the pool-error
                // arm above). Not buffered either: without a pubkey the vote can never verify.
                tracing::warn!(
                    attestor = %vote.attestor,
                    height = vote.height,
                    "👤 unknown attestor at no-local height — dropping"
                );
                Acceptance::Ignore
            } else if !worth_buffering(shared, vote.height) {
                tracing::debug!(
                    digest = ?vote.digest,
                    height = vote.height,
                    "🚮 no-local vote outside producible window — dropping"
                );
                Acceptance::Ignore
            } else {
                admit_pending_vote(
                    pending_votes,
                    max_pending_per_height,
                    peers_by_attestor,
                    source,
                    vote,
                )
            }
        }
        VerifyResult::DivergentDigest => {
            tracing::warn!(digest = ?vote.digest, height = vote.height, "↯ divergent digest from peer");
            // We cannot verify this vote: its BLS signature is over the *signer's* local
            // AttestationData at this height, which differs from ours (that's what makes the
            // digest divergent), so we have no trustworthy bytes to check it against. `Ignore`
            // (not `Reject`) because a genuine source-chain fork is not provable misbehavior —
            // same stance as the `UnknownAttestor` arm below.
            //
            // NOTE: `Ignore` does NOT re-propagate the message (only `Accept` does, under
            // gossipsub's `validate_messages()` — this dead-ends the vote here). That is
            // deliberate: forwarding a payload we could not verify would relay unverified data
            // (spam amplification) and let a peer get a forged divergent vote propagated in an
            // honest attestor's name. It is also not admitted to the pool for the same reason —
            // an unverifiable vote must not seed equivocation state that could frame its
            // claimed signer. Legitimate fork votes still reach every node that shares the
            // divergent digest (and can therefore verify + `Accept`+forward them): in the
            // bootnode-meshed validator set the same-digest holders are mutually reachable
            // without transiting a divergent-digest node, so quorum still forms on each side.
            Acceptance::Ignore
        }
        VerifyResult::BadSignature => {
            tracing::warn!(digest = ?vote.digest, height = vote.height, "🔏 bad bls sig");
            Acceptance::Reject
        }
        VerifyResult::WrongChain => {
            tracing::warn!(?vote.chain_key, "🌐 wrong chain key");
            Acceptance::Reject
        }
        VerifyResult::UnknownAttestor => {
            // Ignore, NOT Reject — same reasoning as the no-local unknown-attestor arm: this
            // verdict reflects our local bls_store view, not provable message invalidity.
            tracing::warn!(attestor = %vote.attestor, "👤 unknown attestor — dropping");
            Acceptance::Ignore
        }
    };

    (acceptance, learned)
}

/// Admit a `NoLocal` vote into the pending buffer, enforcing one slot per `(height, attestor)`
/// and — when we've already learned the claimed attestor's peer id — that the authenticated
/// gossipsub publisher matches it. Returns the gossip acceptance to report:
///
/// * `Reject` — the authenticated publisher does not match the known peer binding for the
///   claimed attestor (spoofed identity; feeds the peer-scoring penalty).
/// * `Ignore` — buffered (or dropped on a full/contested slot); we never propagate a vote we
///   haven't BLS-verified yet.
fn admit_pending_vote(
    pending_votes: &mut PendingVotes,
    max_pending_per_height: usize,
    peers_by_attestor: &[(AttestorId, libp2p::PeerId)],
    source: Option<libp2p::PeerId>,
    vote: Vote,
) -> Acceptance {
    let bound_peer = peers_by_attestor
        .iter()
        .find(|(a, _)| *a == vote.attestor)
        .map(|(_, p)| *p);

    let authenticated = match (bound_peer, source) {
        // Known binding and the signed publisher matches — authenticated claim.
        (Some(bound), Some(src)) if src == bound => true,
        // Known binding but the publisher differs (or is missing) — spoofed identity claim.
        (Some(bound), _) => {
            tracing::warn!(
                attestor = %vote.attestor,
                height = vote.height,
                expected_peer = %bound,
                source = ?source,
                "🎭 pending vote claims an attestor bound to a different peer — rejecting"
            );
            return Acceptance::Reject;
        }
        // No binding learned yet — can't authenticate the claim.
        (None, _) => false,
    };

    let entry = pending_votes.entry(vote.height).or_default();
    if entry.contains_key(&vote.attestor) {
        // An authenticated vote replaces whatever occupied the slot (e.g. an earlier spoof
        // that raced in before we had the binding). An unauthenticated one never displaces
        // an existing entry — first-come wins so a flooder can't overwrite a real vote.
        if authenticated {
            entry.insert(vote.attestor.clone(), vote);
        } else {
            tracing::debug!(
                digest = ?vote.digest,
                height = vote.height,
                "🕳️ pending slot already taken for this attestor — dropping"
            );
        }
    } else if entry.len() < max_pending_per_height {
        tracing::debug!(
            digest = ?vote.digest,
            height = vote.height,
            queued = entry.len() + 1,
            "🕳️ no local data yet — queuing vote"
        );
        entry.insert(vote.attestor.clone(), vote);
    } else {
        tracing::warn!(
            digest = ?vote.digest,
            height = vote.height,
            cap = max_pending_per_height,
            "🕳️ pending buffer full — dropping vote"
        );
    }
    Acceptance::Ignore
}

/// Whether a `NoLocal` vote at `height` is worth buffering. It must sit on the local attestation
/// schedule (so matching local data could ever exist for it) and inside the same admission window
/// the pool itself uses. Anything off-schedule or out-of-window can never become verifiable and
/// would only grow the pending buffer — the call sites drop those.
fn worth_buffering(shared: &Arc<Shared>, height: attestor_primitives::Height) -> bool {
    let interval = shared.attestation_interval().get();
    let max_catchup = shared.max_catchup().get();
    let finalized = shared.latest_finalized_rx.borrow().map(|info| info.height);
    is_bufferable(
        height,
        shared.genesis,
        interval,
        max_catchup,
        finalized,
        shared.start_height,
    )
}

/// Pure predicate behind [`worth_buffering`], split out so the schedule/window logic is unit
/// testable without a full [`Shared`].
fn is_bufferable(
    height: attestor_primitives::Height,
    genesis: attestor_primitives::Height,
    interval: attestor_primitives::Height,
    max_catchup: attestor_primitives::Height,
    finalized: Option<attestor_primitives::Height>,
    start_height: attestor_primitives::Height,
) -> bool {
    // `StreamAttestation` emits the genesis attestation once and every later attestation at an
    // absolute multiple of the interval (`next - next % interval`). A height that is neither can
    // never gain local data.
    if height != genesis && height % interval != 0 {
        return false;
    }
    // Bound to the window the pool would admit (see `ValidateQuorum::height_admissible`):
    // above the last finalized attestation and within `max_catchup` *blocks* of it
    // (`max_catchup` is a block-count bound, matching the runtime storage docs and
    // `StreamAttestation` — production never emits further ahead than that). `max(interval)`
    // keeps the next interval-aligned target admissible when the configured bound is smaller
    // than the interval. Anchoring on the finalized height (not local production, which only
    // climbs) keeps the buffer bounded.
    let window = max_catchup.max(interval);
    match finalized {
        Some(finalized) => height > finalized && height <= finalized.saturating_add(window),
        // Nothing attested on-chain yet. The floor must be *inclusive* of `start_height` here:
        // at a cold-start bootstrap `start_height == genesis`, and the genesis votes peers
        // gossip are exactly the ones we must hold until our own genesis data is built. An
        // exclusive floor (the old `unwrap_or(start_height)` collapse) dropped them — and a
        // gossipsub `Ignore` is terminal (the message is marked seen and never redelivered;
        // production emits its genesis vote once), so staggered/parallel cold starts could
        // leave every attestor short of genesis quorum.
        None => height >= start_height && height <= start_height.saturating_add(window),
    }
}

/// Re-process a vote that was previously queued because local data was missing. Local data
/// should now be present (production just signaled us); this is the same verify + pool-send
/// pipeline `handle_vote_msg` runs, minus the queueing fallback.
async fn retry_pending_vote(shared: &Arc<Shared>, vote: Vote) {
    let local = shared.proof_cache.local_data(vote.height);
    let pubkey = shared.bls_store.pubkey(vote.attestor.account_id());
    match verify_vote(&vote, shared.chain_key, local.as_ref(), pubkey.as_ref()) {
        VerifyResult::Accept => {
            if let Some(Err(err)) = shared.pool_send.send(vote.clone()) {
                err.log_error(vote.digest);
            }
        }
        // Anything other than Accept at retry time is a real problem (divergent digest from
        // a peer who saw a different eth block, bad sig, etc.) — log and drop. NoLocal here
        // would mean production raced its own signal; harmless, just drop.
        result => {
            tracing::debug!(
                digest = ?vote.digest,
                height = vote.height,
                ?result,
                "🕳️ pending vote no longer admissible at retry"
            );
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn insufficient_peers_is_expected_only_for_a_solo_attestor() {
        use libp2p::gossipsub::PublishError;
        assert!(super::publish_failure_is_expected(
            &PublishError::InsufficientPeers,
            true
        ));
        // In a real set the same error is the stalled-mesh signal and must stay a warning.
        assert!(!super::publish_failure_is_expected(
            &PublishError::InsufficientPeers,
            false
        ));
        // Other failures are never expected, solo or not.
        assert!(!super::publish_failure_is_expected(
            &PublishError::MessageTooLarge,
            true
        ));
    }

    use super::{
        apply_peer_registry, deny_decision, drain_pending_votes, ensure_local_peer_key,
        is_bufferable, note_attestor_peer, note_verified_vote_peer, update_peer_registry,
        PendingVotes, ReobsAdmission, ReobsKey, REOBS_DEDUP_WINDOW, REOBS_PER_PEER_CAPACITY,
    };
    use attestor_primitives::AttestorId;
    use std::time::{Duration, Instant};

    #[test]
    fn reobs_admission_enforces_per_peer_and_refills() {
        let t0 = Instant::now();
        let mut a = ReobsAdmission::new(t0);
        let p = libp2p::PeerId::random();
        // Per-peer capacity is exhausted, then denied at the same instant…
        for i in 0..REOBS_PER_PEER_CAPACITY as usize {
            assert!(
                a.admit(&rkey(i as u8), p, t0),
                "request {i} within per-peer capacity"
            );
        }
        assert!(!a.admit(&rkey(100), p, t0), "over per-peer capacity");
        // …a different peer has its own bucket (shares only the global allowance)…
        let q = libp2p::PeerId::random();
        assert!(a.admit(&rkey(101), q, t0));
        // …and the per-peer bucket refills over time (1 token/sec).
        assert!(a.admit(&rkey(102), p, t0 + Duration::from_secs(1)));
    }

    fn rkey(n: u8) -> ReobsKey {
        ReobsKey {
            message_id: alloy::primitives::B256::from([n; 32]),
            tx_hash: alloy::primitives::B256::from([n; 32]),
            block_height: u64::from(n),
        }
    }

    #[test]
    fn reobs_admission_replays_cost_no_budget() {
        // CSUB-2138: the same forged tuple replayed from many peers is admitted once, and the
        // replays charge neither their per-peer bucket nor the shared global bucket.
        let t0 = Instant::now();
        let mut a = ReobsAdmission::new(t0);
        let forged = rkey(1);
        assert!(a.admit(&forged, libp2p::PeerId::random(), t0));
        for _ in 0..100 {
            assert!(!a.admit(&forged, libp2p::PeerId::random(), t0));
        }
        // Global capacity (20) is intact: 19 more distinct requests from distinct peers fit.
        for n in 2..=20u8 {
            assert!(
                a.admit(&rkey(n), libp2p::PeerId::random(), t0),
                "request {n}"
            );
        }
        assert!(
            !a.admit(&rkey(21), libp2p::PeerId::random(), t0),
            "global now empty"
        );
        // After the dedup window the same tuple is admissible again (a genuine retry).
        assert!(a.admit(&forged, libp2p::PeerId::random(), t0 + REOBS_DEDUP_WINDOW));
    }

    fn wire_request(n: u8) -> write_ability::envelope::ReobservationRequest {
        write_ability::envelope::ReobservationRequest {
            chain_key: 1,
            message_id: [n; 32],
            tx_hash: [n; 32],
            block_height: u64::from(n),
        }
    }

    #[test]
    fn a_copy_the_queue_rejects_does_not_shadow_the_genuine_request() {
        use super::{intake_reobservation, QueuePush};
        use libp2p::gossipsub::MessageAcceptance;
        // The queue lets each peer hold one request, and the attacker's peer already holds one.
        let t0 = Instant::now();
        let mut admission = ReobsAdmission::new(t0);
        let queue = crate::tasks::write_ability::reobservation::ReobsQueue::new(10, 1);
        let (attacker, honest) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        assert_eq!(queue.push(attacker, wire_request(100)), QueuePush::Queued);

        // A copy of the genuine request arrives first via the congested peer: rejected, not
        // propagated, and not remembered.
        assert!(matches!(
            intake_reobservation(&mut admission, &queue, attacker, wire_request(1), t0),
            MessageAcceptance::Ignore
        ));
        // The genuine request via another neighbour, inside the replay window, is still handled.
        assert!(matches!(
            intake_reobservation(&mut admission, &queue, honest, wire_request(1), t0),
            MessageAcceptance::Accept
        ));
        // Once it is queued, further identical copies are dropped for free as replays.
        assert!(matches!(
            intake_reobservation(
                &mut admission,
                &queue,
                libp2p::PeerId::random(),
                wire_request(1),
                t0
            ),
            MessageAcceptance::Ignore
        ));
    }

    #[test]
    fn rejected_copies_cost_no_admission_budget() {
        use super::{intake_reobservation, QueuePush, REOBS_GLOBAL_CAPACITY};
        use libp2p::gossipsub::MessageAcceptance;
        let t0 = Instant::now();
        let mut admission = ReobsAdmission::new(t0);
        // A one-slot queue that is already full, so every new request is turned away.
        let queue = crate::tasks::write_ability::reobservation::ReobsQueue::new(1, 1);
        assert_eq!(
            queue.push(libp2p::PeerId::random(), wire_request(100)),
            QueuePush::Queued
        );
        let attacker = libp2p::PeerId::random();
        for i in 0..100 {
            let sender = if i % 2 == 0 {
                attacker
            } else {
                libp2p::PeerId::random()
            };
            assert!(matches!(
                intake_reobservation(&mut admission, &queue, sender, wire_request(1), t0),
                MessageAcceptance::Ignore
            ));
        }
        // Neither the shared bucket nor the repeat sender's own bucket was charged...
        assert!(admission.global.tokens >= REOBS_GLOBAL_CAPACITY);
        assert!(admission.per_peer[&attacker].tokens >= REOBS_PER_PEER_CAPACITY);
        // ...and the tuple was not remembered, so it is not blocked as a replay either.
        assert!(admission.check(&rkey(1), libp2p::PeerId::random(), t0));
    }

    #[test]
    fn a_queued_request_is_charged_once_and_its_copies_are_dropped_free() {
        use super::{intake_reobservation, REOBS_GLOBAL_CAPACITY};
        use libp2p::gossipsub::MessageAcceptance;
        let t0 = Instant::now();
        let mut admission = ReobsAdmission::new(t0);
        let queue = crate::tasks::write_ability::reobservation::ReobsQueue::new(10, 10);
        let first = libp2p::PeerId::random();
        assert!(matches!(
            intake_reobservation(&mut admission, &queue, first, wire_request(1), t0),
            MessageAcceptance::Accept
        ));
        assert_eq!(admission.global.tokens, REOBS_GLOBAL_CAPACITY - 1.0);
        for _ in 0..50 {
            assert!(matches!(
                intake_reobservation(
                    &mut admission,
                    &queue,
                    libp2p::PeerId::random(),
                    wire_request(1),
                    t0
                ),
                MessageAcceptance::Ignore
            ));
        }
        assert_eq!(admission.global.tokens, REOBS_GLOBAL_CAPACITY - 1.0);
    }

    #[test]
    fn only_a_queued_reobservation_request_keeps_propagating() {
        use super::{reobs_push_acceptance, QueuePush};
        use libp2p::gossipsub::MessageAcceptance;
        assert!(matches!(
            reobs_push_acceptance(QueuePush::Queued),
            MessageAcceptance::Accept
        ));
        for dropped in [QueuePush::Duplicate, QueuePush::PeerFull, QueuePush::Full] {
            assert!(
                matches!(reobs_push_acceptance(dropped), MessageAcceptance::Ignore),
                "{dropped:?} must not be propagated"
            );
        }
    }

    fn att(n: u8) -> AttestorId {
        AttestorId::from_public([n; 32])
    }

    pub(super) fn registry(
        required: bool,
        attestors: Vec<(AttestorId, Option<libp2p::PeerId>)>,
    ) -> crate::peer_registry::Registry {
        crate::peer_registry::Registry {
            genesis_hash: Default::default(),
            registration_required: required,
            attestors,
            waiting_attestors: vec![],
            finalized_hash: Default::default(),
            finalized_number: 0,
        }
    }

    #[test]
    fn sole_registered_public_seed_bootstraps_during_rollout_and_enforcement() {
        let local = libp2p::PeerId::random();
        for required in [false, true] {
            let registry = registry(required, vec![(att(1), Some(local))]);
            // Startup and admission consume the same finalized-registry decision. This also
            // covers a restart after every other committee member has been chilled.
            assert!(registry.has_bootstrap_peers(local));
            let mut admission = super::admission::Behaviour::new(2);
            super::configure_bootstrap_admission(
                &mut admission,
                &std::collections::HashSet::new(),
                &[],
                local,
                false,
                true,
                &registry,
            )
            .unwrap();
            assert!(!admission.is_reserved(&local));
        }
    }

    #[test]
    fn incomplete_committee_cannot_bootstrap_a_public_seed_with_only_its_own_key() {
        let local = libp2p::PeerId::random();
        for attestors in [
            vec![],
            vec![(att(1), None)],
            vec![(att(1), Some(local)), (att(2), None)],
        ] {
            let registry = registry(false, attestors);
            assert!(!registry.has_bootstrap_peers(local));
            assert!(super::configure_bootstrap_admission(
                &mut super::admission::Behaviour::new(2),
                &std::collections::HashSet::new(),
                &[],
                local,
                false,
                true,
                &registry,
            )
            .is_err());
        }
    }

    #[test]
    fn public_seed_waits_past_leftover_fallback_peers_until_a_key_finalizes() {
        let local = libp2p::PeerId::random();
        let stale = libp2p::PeerId::random();
        let remote = libp2p::PeerId::random();
        let configured = [local, stale];
        let boot_peers = std::collections::HashSet::new();
        let mut registry = registry(true, vec![]);
        let mut admission = super::admission::Behaviour::new(2);

        assert!(!super::can_bootstrap_seed(
            local,
            false,
            &configured,
            &registry
        ));
        let error = super::configure_bootstrap_admission(
            &mut admission,
            &boot_peers,
            &configured,
            local,
            false,
            true,
            &registry,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("finalized committee networking keys"));
        assert!(error.to_string().contains("are ignored"));
        assert!(!admission.is_reserved(&stale));

        // A fresh finalized snapshot, including a waiting candidate, releases the startup wait.
        for waiting in [false, true] {
            registry.attestors.clear();
            registry.waiting_attestors.clear();
            if waiting {
                registry.waiting_attestors.push((att(1), Some(remote)));
            } else {
                registry.attestors.push((att(1), Some(remote)));
            }
            assert!(super::can_bootstrap_seed(
                local,
                false,
                &configured,
                &registry
            ));
            super::configure_bootstrap_admission(
                &mut admission,
                &boot_peers,
                &configured,
                local,
                false,
                true,
                &registry,
            )
            .unwrap();
            assert!(!admission.is_reserved(&stale));
        }
    }

    #[test]
    fn public_seed_fallback_stops_working_when_enforcement_finalizes() {
        let local = libp2p::PeerId::random();
        let remote = libp2p::PeerId::random();
        let mut registry = registry(false, vec![]);
        let mut admission = super::admission::Behaviour::new(2);

        assert!(!super::can_bootstrap_seed(
            local,
            false,
            &[local],
            &registry
        ));
        assert!(super::can_bootstrap_seed(
            local,
            false,
            &[local, remote],
            &registry
        ));
        super::configure_bootstrap_admission(
            &mut admission,
            &std::collections::HashSet::new(),
            &[local, remote],
            local,
            false,
            true,
            &registry,
        )
        .unwrap();
        assert!(admission.is_reserved(&remote));
        assert!(!admission.is_reserved(&local));

        registry.registration_required = true;
        assert!(!super::can_bootstrap_seed(
            local,
            false,
            &[local, remote],
            &registry
        ));
    }

    #[test]
    fn enforced_public_seed_keeps_boot_nodes_without_reserving_fallback_peers() {
        let local = libp2p::PeerId::random();
        let boot = libp2p::PeerId::random();
        let fallback = libp2p::PeerId::random();
        let registry = registry(true, vec![]);
        let mut admission = super::admission::Behaviour::new(2);

        assert!(super::can_bootstrap_seed(
            local,
            true,
            &[fallback],
            &registry
        ));
        super::configure_bootstrap_admission(
            &mut admission,
            &std::collections::HashSet::from([boot]),
            &[fallback],
            local,
            false,
            true,
            &registry,
        )
        .unwrap();
        assert!(admission.is_reserved(&boot));
        assert!(!admission.is_reserved(&fallback));
    }

    fn swarm() -> libp2p::Swarm<super::behavior::P2PBehavior> {
        let topic = libp2p::gossipsub::IdentTopic::new("1/attest");
        libp2p::SwarmBuilder::with_new_identity()
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )
            .unwrap()
            .with_behaviour(|key| super::behavior::P2PBehavior::new(key, false, &[&topic], &[]))
            .unwrap()
            .build()
    }

    #[tokio::test]
    async fn registered_identity_cannot_be_rebound_by_a_republished_valid_vote() {
        let mut swarm = swarm();
        let (registered, attacker) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        let registry = registry(false, vec![(att(1), Some(registered))]);
        let mut bindings = Vec::new();
        apply_peer_registry(&mut swarm, &mut bindings, &registry, &[], &mut 0);
        note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &registry,
            att(1),
            attacker,
        );
        assert_eq!(bindings, vec![(att(1), registered)]);
        assert!(!swarm.behaviour().admission.is_reserved(&attacker));
    }

    #[tokio::test]
    async fn waiting_peer_is_admitted_before_election_and_eviction_revokes_cached_reservations() {
        use libp2p::swarm::{ConnectionId, NetworkBehaviour as _};
        let mut swarm = swarm();
        // Saturate the public pool without opening real sockets.
        swarm.behaviour_mut().admission = super::admission::Behaviour::new(0);
        let (member, old, republisher) = (
            libp2p::PeerId::random(),
            libp2p::PeerId::random(),
            libp2p::PeerId::random(),
        );
        let mut registry = registry(false, vec![]);
        registry.waiting_attestors = vec![(att(1), Some(member))];
        let mut bindings = vec![(att(1), old)];
        apply_peer_registry(&mut swarm, &mut bindings, &registry, &[member], &mut 0);
        assert!(
            bindings.is_empty(),
            "waiting must not enter the inactive-voter deny gate"
        );
        let addr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();
        assert!(swarm
            .behaviour_mut()
            .admission
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(1),
                member,
                &addr,
                &addr
            )
            .is_ok());
        assert!(swarm
            .behaviour_mut()
            .admission
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(2),
                republisher,
                &addr,
                &addr
            )
            .is_err());

        // Election and a valid BLS vote can precede the next registry refresh. Keep the
        // registered transport identity even if another peer republishes that vote.
        note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &registry,
            att(1),
            republisher,
        );
        assert_eq!(bindings, vec![(att(1), member)]);
        super::handle_peer_deactivated(
            &mut swarm,
            &mut bindings,
            &registry,
            &mut 0,
            &crate::shared::PeerDeactivated {
                attestor: att(1),
                finalized_number: 1,
            },
            |_| false,
        );
        assert!(!swarm.behaviour().admission.is_reserved(&member));
        assert!(swarm
            .behaviour_mut()
            .admission
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(3),
                member,
                &addr,
                &addr
            )
            .is_err());
    }

    #[tokio::test]
    async fn registry_refresh_keeps_a_deactivated_fallback_peer_revoked() {
        let mut swarm = swarm();
        let member = libp2p::PeerId::random();
        let mut bindings = vec![(att(1), member)];
        let mut minimum = 0;
        let current = registry(false, vec![(att(1), None)]);
        apply_peer_registry(&mut swarm, &mut bindings, &current, &[member], &mut minimum);
        assert!(swarm.behaviour().admission.is_reserved(&member));

        super::handle_peer_deactivated(
            &mut swarm,
            &mut bindings,
            &current,
            &mut minimum,
            &crate::shared::PeerDeactivated {
                attestor: att(1),
                finalized_number: 1,
            },
            |_| false,
        );
        assert!(!swarm.behaviour().admission.is_reserved(&member));

        // The next finalized snapshot no longer lists the attestor: the manual fallback must
        // not hand the evicted peer its reservation back.
        let mut after = registry(false, vec![]);
        after.finalized_number = 1;
        assert!(apply_peer_registry(
            &mut swarm,
            &mut bindings,
            &after,
            &[member],
            &mut minimum
        ));
        assert!(!swarm.behaviour().admission.is_reserved(&member));

        // Rejoining restores the fallback reservation.
        let mut rejoined = registry(false, vec![(att(1), None)]);
        rejoined.finalized_number = 2;
        assert!(apply_peer_registry(
            &mut swarm,
            &mut bindings,
            &rejoined,
            &[member],
            &mut minimum
        ));
        assert!(swarm.behaviour().admission.is_reserved(&member));
    }

    #[tokio::test]
    async fn waiting_deactivation_uses_registered_identity_before_first_vote() {
        for required in [false, true] {
            for other_active in [false, true] {
                let mut swarm = swarm();
                let member = libp2p::PeerId::random();
                let mut registry = registry(required, vec![]);
                registry.waiting_attestors = vec![(att(1), Some(member))];
                let mut bindings = Vec::new();
                apply_peer_registry(&mut swarm, &mut bindings, &registry, &[member], &mut 0);
                assert!(bindings.is_empty());
                assert!(swarm.behaviour().admission.is_reserved(&member));
                if other_active {
                    note_attestor_peer(&mut bindings, att(2), member);
                }
                let is_active = |id: &AttestorId| other_active && *id == att(2);
                super::handle_peer_deactivated(
                    &mut swarm,
                    &mut bindings,
                    &registry,
                    &mut 0,
                    &crate::shared::PeerDeactivated {
                        attestor: att(1),
                        finalized_number: 1,
                    },
                    is_active,
                );
                assert!(bindings.contains(&(att(1), member)));
                assert_eq!(
                    swarm.behaviour().admission.is_reserved(&member),
                    other_active
                );
                assert_eq!(
                    deny_decision(
                        bindings
                            .iter()
                            .filter(|(_, peer)| *peer == member)
                            .map(|(id, _)| id),
                        is_active,
                    ),
                    !other_active
                );
            }
        }
    }

    #[tokio::test]
    async fn stale_registry_cannot_undo_deactivation_but_later_reactivation_can() {
        for required in [false, true] {
            let mut swarm = swarm();
            let member = libp2p::PeerId::random();
            let mut current = registry(required, vec![]);
            current.finalized_number = 10;
            current.waiting_attestors = vec![(att(1), Some(member))];
            let mut bindings = Vec::new();
            let mut minimum = 0;
            assert!(apply_peer_registry(
                &mut swarm,
                &mut bindings,
                &current,
                &[member],
                &mut minimum
            ));
            assert!(swarm.behaviour().admission.is_reserved(&member));

            let removal = crate::shared::PeerDeactivated {
                attestor: att(1),
                finalized_number: 20,
            };
            super::handle_peer_deactivated(
                &mut swarm,
                &mut bindings,
                &current,
                &mut minimum,
                &removal,
                |_| false,
            );
            assert_eq!(minimum, 20);
            assert!(!swarm.behaviour().admission.is_reserved(&member));

            // A refresh can be newer than our last registry but still predate the removal.
            let mut next = current.clone();
            next.finalized_number = 15;
            assert!(!apply_peer_registry(
                &mut swarm,
                &mut bindings,
                &next,
                &[member],
                &mut minimum
            ));
            assert_eq!(bindings, vec![(att(1), member)]);
            assert!(!swarm.behaviour().admission.is_reserved(&member));

            next.finalized_number = 20;
            next.waiting_attestors.clear();
            assert!(apply_peer_registry(
                &mut swarm,
                &mut bindings,
                &next,
                &[],
                &mut minimum
            ));
            assert!(!swarm.behaviour().admission.is_reserved(&member));

            // Reactivation clears the deny binding and restores capacity. A delayed/backfilled
            // removal notification must not undo this already-finalized post-block state.
            next.finalized_number = 21;
            next.waiting_attestors = vec![(att(1), Some(member))];
            assert!(apply_peer_registry(
                &mut swarm,
                &mut bindings,
                &next,
                &[member],
                &mut minimum
            ));
            current = next;
            super::handle_peer_deactivated(
                &mut swarm,
                &mut bindings,
                &current,
                &mut minimum,
                &removal,
                |_| false,
            );
            assert!(bindings.is_empty());
            assert!(swarm.behaviour().admission.is_reserved(&member));
        }
    }

    #[tokio::test]
    async fn unchanged_reservations_still_advance_the_removal_watermark() {
        let mut swarm = swarm();
        let member = libp2p::PeerId::random();
        let mut current = registry(true, vec![(att(1), Some(member))]);
        current.finalized_number = 10;
        let mut bindings = Vec::new();
        let mut minimum = 0;
        apply_peer_registry(&mut swarm, &mut bindings, &current, &[], &mut minimum);
        let mut next = current.clone();
        next.finalized_number = 20;
        next.finalized_hash = cc_client::H256::repeat_byte(1);
        assert!(current.same_reservations(&next));
        assert!(update_peer_registry(
            &mut swarm,
            &mut bindings,
            &current,
            &next,
            &[],
            &mut minimum
        ));
        assert_eq!(minimum, 20);
        // A delayed event already covered by the newer view must not evict its active peer.
        super::handle_peer_deactivated(
            &mut swarm,
            &mut bindings,
            &next,
            &mut minimum,
            &crate::shared::PeerDeactivated {
                attestor: att(1),
                finalized_number: 15,
            },
            |_| false,
        );
        assert!(swarm.behaviour().admission.is_reserved(&member));
    }

    #[tokio::test]
    async fn coalesced_removal_and_reactivation_reconcile_even_when_membership_is_identical() {
        for required in [false, true] {
            let mut swarm = swarm();
            let member = libp2p::PeerId::random();
            let mut current = registry(required, vec![]);
            current.finalized_number = 10;
            current.waiting_attestors = vec![(att(1), Some(member))];
            let mut bindings = Vec::new();
            let mut minimum = 0;
            apply_peer_registry(&mut swarm, &mut bindings, &current, &[member], &mut minimum);
            let removal = crate::shared::PeerDeactivated {
                attestor: att(1),
                finalized_number: 20,
            };
            super::handle_peer_deactivated(
                &mut swarm,
                &mut bindings,
                &current,
                &mut minimum,
                &removal,
                |_| false,
            );
            assert!(!swarm.behaviour().admission.is_reserved(&member));
            let mut next = current.clone();
            next.finalized_number = 15;
            assert!(!update_peer_registry(
                &mut swarm,
                &mut bindings,
                &current,
                &next,
                &[member],
                &mut minimum
            ));
            assert!(!swarm.behaviour().admission.is_reserved(&member));

            next.finalized_number = 21;
            assert!(current.same_reservations(&next));
            assert!(update_peer_registry(
                &mut swarm,
                &mut bindings,
                &current,
                &next,
                &[member],
                &mut minimum
            ));
            assert!(bindings.is_empty());
            assert!(swarm.behaviour().admission.is_reserved(&member));
            super::handle_peer_deactivated(
                &mut swarm,
                &mut bindings,
                &next,
                &mut minimum,
                &removal,
                |_| false,
            );
            assert!(swarm.behaviour().admission.is_reserved(&member));
        }
    }

    fn in_routing_table(
        swarm: &mut libp2p::Swarm<super::behavior::P2PBehavior>,
        peer: &libp2p::PeerId,
    ) -> bool {
        swarm
            .behaviour_mut()
            .kad
            .kbuckets()
            .any(|bucket| bucket.iter().any(|entry| entry.node.key.preimage() == peer))
    }

    #[tokio::test]
    async fn deny_gate_eviction_keeps_a_finalized_committee_reservation() {
        for required in [false, true] {
            let mut swarm = swarm();
            let member = libp2p::PeerId::random();
            let mut current = registry(required, vec![(att(1), Some(member))]);
            current.finalized_number = 10;
            let mut bindings = Vec::new();
            let mut minimum = 0;
            apply_peer_registry(&mut swarm, &mut bindings, &current, &[], &mut minimum);
            // The registry can finalize an election before production refreshes the BLS store,
            // so the deny gate briefly reports the newly elected member as inactive.
            assert!(deny_decision(
                bindings
                    .iter()
                    .filter(|(_, peer)| *peer == member)
                    .map(|(id, _)| id),
                |_| false
            ));
            super::evict_peer(&mut swarm, member);
            // Identical membership skips the rebuild, so the eviction must not have dropped it.
            let mut next = current.clone();
            next.finalized_number = 11;
            update_peer_registry(
                &mut swarm,
                &mut bindings,
                &current,
                &next,
                &[],
                &mut minimum,
            );
            assert!(swarm.behaviour().admission.is_reserved(&member));
        }
    }

    #[tokio::test]
    async fn delayed_deactivation_evicts_a_peer_readmitted_before_the_bls_refresh() {
        for required in [false, true] {
            let mut swarm = swarm();
            let chilled = libp2p::PeerId::random();
            let mut current = registry(required, vec![(att(1), Some(chilled))]);
            current.finalized_number = 10;
            let mut bindings = Vec::new();
            let mut minimum = 0;
            apply_peer_registry(&mut swarm, &mut bindings, &current, &[], &mut minimum);
            // The registry finalizes the chill first and releases the slot.
            let mut next = registry(required, vec![]);
            next.finalized_number = 20;
            update_peer_registry(
                &mut swarm,
                &mut bindings,
                &current,
                &next,
                &[],
                &mut minimum,
            );
            assert!(!swarm.behaviour().admission.is_reserved(&chilled));
            // The peer is rediscovered while the stale BLS store still reports it active.
            assert!(!deny_decision(
                bindings
                    .iter()
                    .filter(|(_, peer)| *peer == chilled)
                    .map(|(id, _)| id),
                |_| true
            ));
            swarm
                .behaviour_mut()
                .kad
                .add_address(&chilled, "/ip4/127.0.0.1/tcp/9000".parse().unwrap());
            assert!(in_routing_table(&mut swarm, &chilled));
            super::handle_peer_deactivated(
                &mut swarm,
                &mut bindings,
                &next,
                &mut minimum,
                &crate::shared::PeerDeactivated {
                    attestor: att(1),
                    finalized_number: 20,
                },
                |_| false,
            );
            assert!(!in_routing_table(&mut swarm, &chilled));
            assert_eq!(minimum, 20);
        }
    }

    #[tokio::test]
    async fn registry_rotation_replaces_legacy_and_configured_reservations() {
        let mut swarm = swarm();
        let (old, new, boot) = (
            libp2p::PeerId::random(),
            libp2p::PeerId::random(),
            libp2p::PeerId::random(),
        );
        swarm.behaviour_mut().admission.pin(boot);
        let mut bindings = Vec::new();
        let legacy = registry(false, vec![(att(1), None)]);
        note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &legacy,
            att(1),
            old,
        );
        apply_peer_registry(
            &mut swarm,
            &mut bindings,
            &registry(true, vec![(att(1), Some(new))]),
            &[old],
            &mut 0,
        );
        assert_eq!(bindings, vec![(att(1), new)]);
        assert!(!swarm.behaviour().admission.is_reserved(&old));
        assert!(swarm.behaviour().admission.is_reserved(&new));
        assert!(swarm.behaviour().admission.is_reserved(&boot));
    }

    /// `old` is bound to a legacy member (att 1) and to a former member (att 2) whose binding is
    /// kept for the deny gate. Returns the swarm, bindings and registry with `old` in Kademlia.
    fn co_bound_peer(
        extra_members: Vec<AttestorId>,
        old: libp2p::PeerId,
    ) -> (
        libp2p::Swarm<super::behavior::P2PBehavior>,
        Vec<(AttestorId, libp2p::PeerId)>,
        crate::peer_registry::Registry,
    ) {
        let mut swarm = swarm();
        let mut bindings = vec![(att(2), old)];
        let members = std::iter::once(att(1)).chain(extra_members);
        let registry = registry(false, members.map(|id| (id, None)).collect());
        for (id, _) in registry.attestors.clone() {
            note_verified_vote_peer(
                &mut swarm.behaviour_mut().admission,
                &mut bindings,
                &registry,
                id,
                old,
            );
        }
        swarm
            .behaviour_mut()
            .kad
            .add_address(&old, "/ip4/127.0.0.1/tcp/9000".parse().unwrap());
        assert!(in_routing_table(&mut swarm, &old));
        (swarm, bindings, registry)
    }

    #[tokio::test]
    async fn vote_replacement_evicts_a_peer_that_only_maps_to_former_members() {
        let (old, new) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        let (mut swarm, mut bindings, registry) = co_bound_peer(vec![], old);

        let released = note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &registry,
            att(1),
            new,
        );
        assert_eq!(released, Some(old));
        super::evict_replaced_peer(&mut swarm, &bindings, &registry, old);

        assert!(!in_routing_table(&mut swarm, &old));
        assert!(!swarm.behaviour().admission.is_reserved(&old));
        assert!(swarm.behaviour().admission.is_reserved(&new));
    }

    #[tokio::test]
    async fn vote_replacement_keeps_a_peer_another_active_member_still_uses() {
        let (old, new) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        let (mut swarm, mut bindings, registry) = co_bound_peer(vec![att(3)], old);

        let released = note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &registry,
            att(1),
            new,
        );
        assert_eq!(released, None);

        assert!(in_routing_table(&mut swarm, &old));
        assert!(swarm.behaviour().admission.is_reserved(&old));
    }

    #[tokio::test]
    async fn vote_replacement_keeps_a_peer_that_holds_another_reservation() {
        let (old, new) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        let (mut swarm, mut bindings, registry) = co_bound_peer(vec![], old);
        swarm.behaviour_mut().admission.pin(old);

        let released = note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &registry,
            att(1),
            new,
        );
        assert_eq!(released, Some(old));
        super::evict_replaced_peer(&mut swarm, &bindings, &registry, old);

        assert!(in_routing_table(&mut swarm, &old));
        assert!(swarm.behaviour().admission.is_reserved(&old));
    }

    /// The deny gate only targets peers tied to a former member, so a peer that no attestor maps
    /// to any more is left as an ordinary peer.
    #[tokio::test]
    async fn vote_replacement_leaves_an_unmapped_peer_alone() {
        let (old, new) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        let (mut swarm, mut bindings, registry) = co_bound_peer(vec![], old);
        bindings.retain(|(id, _)| *id != att(2));

        let released = note_verified_vote_peer(
            &mut swarm.behaviour_mut().admission,
            &mut bindings,
            &registry,
            att(1),
            new,
        );
        assert_eq!(released, Some(old));
        super::evict_replaced_peer(&mut swarm, &bindings, &registry, old);

        assert!(in_routing_table(&mut swarm, &old));
    }

    #[tokio::test]
    async fn deactivated_member_loses_capacity_and_keeps_a_bounded_deny_binding() {
        let mut swarm = swarm();
        let old = libp2p::PeerId::random();
        let mut bindings = Vec::new();
        apply_peer_registry(
            &mut swarm,
            &mut bindings,
            &registry(true, vec![(att(1), Some(old))]),
            &[],
            &mut 0,
        );
        apply_peer_registry(
            &mut swarm,
            &mut bindings,
            &registry(true, vec![]),
            &[],
            &mut 0,
        );
        assert!(!swarm.behaviour().admission.is_reserved(&old));
        assert!(deny_decision(
            bindings
                .iter()
                .filter(|(_, peer)| *peer == old)
                .map(|(id, _)| id),
            |_| false
        ));
    }

    #[test]
    fn an_instance_stops_if_its_own_finalized_key_changes() {
        let (old, new) = (libp2p::PeerId::random(), libp2p::PeerId::random());
        let registry = registry(true, vec![(att(1), Some(new))]);
        assert!(ensure_local_peer_key(&registry, &att(1), old).is_err());
        assert!(ensure_local_peer_key(&registry, &att(1), new).is_ok());
    }

    // ------------------------------* attestor peer map *------------------------------ //

    #[test]
    fn note_attestor_peer_reports_the_replaced_peer_id() {
        let mut map = Vec::new();
        let (first, second) = (libp2p::PeerId::random(), libp2p::PeerId::random());

        assert_eq!(note_attestor_peer(&mut map, att(1), first), None);
        assert_eq!(note_attestor_peer(&mut map, att(1), first), None);
        assert_eq!(note_attestor_peer(&mut map, att(1), second), Some(first));
        assert_eq!(map, vec![(att(1), second)]);
    }

    // ------------------------------* deny-list decision *------------------------------ //

    #[test]
    fn unmapped_peer_is_never_denied() {
        // No attestor mapped to the peer → we can't tie it to a removed attestor → allow.
        let active: Vec<AttestorId> = vec![];
        let mapped: [AttestorId; 0] = [];
        assert!(!deny_decision(mapped.iter(), |a| active.contains(a)));
    }

    #[test]
    fn peer_mapped_only_to_inactive_attestor_is_denied() {
        let active: Vec<AttestorId> = vec![];
        let mapped = [att(1)];
        assert!(deny_decision(mapped.iter(), |a| active.contains(a)));
    }

    #[test]
    fn peer_mapped_to_active_attestor_is_allowed() {
        let active: Vec<AttestorId> = vec![att(1)];
        let mapped = [att(1)];
        assert!(!deny_decision(mapped.iter(), |a| active.contains(a)));
    }

    #[test]
    fn shared_peer_with_one_active_attestor_is_allowed() {
        // Same peer id bound to a chilled id (1) and a re-registered active id (2): the active
        // mapping must win, so a node that re-registers under a new active attestor id keeps
        // connectivity even though its old id is still chilled.
        let active: Vec<AttestorId> = vec![att(2)];
        let mapped = [att(1), att(2)];
        assert!(!deny_decision(mapped.iter(), |a| active.contains(a)));
    }

    #[test]
    fn shared_peer_with_all_inactive_attestors_is_denied() {
        let active: Vec<AttestorId> = vec![];
        let mapped = [att(1), att(2)];
        assert!(deny_decision(mapped.iter(), |a| active.contains(a)));
    }

    // ---------------------------* dial-failure eviction *--------------------------- //

    use super::{note_dial_failure, MAX_DIAL_FAILURES};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn peer_evicts_only_after_max_consecutive_dial_failures() {
        let mut failures = HashMap::new();
        let boot = HashSet::new();
        let peer = libp2p::PeerId::random();
        for _ in 0..MAX_DIAL_FAILURES - 1 {
            assert!(!note_dial_failure(&mut failures, &boot, peer));
        }
        assert!(note_dial_failure(&mut failures, &boot, peer));
    }

    #[test]
    fn eviction_clears_the_counter_for_a_fresh_start() {
        let mut failures = HashMap::new();
        let boot = HashSet::new();
        let peer = libp2p::PeerId::random();
        for _ in 0..MAX_DIAL_FAILURES {
            note_dial_failure(&mut failures, &boot, peer);
        }
        // Counter was cleared on eviction: the peer must survive another full round of failures
        // before evicting again (rediscovery is not punished for the old streak).
        assert!(!failures.contains_key(&peer));
        assert!(!note_dial_failure(&mut failures, &boot, peer));
    }

    #[test]
    fn boot_nodes_are_never_evicted_and_never_tracked() {
        let mut failures = HashMap::new();
        let peer = libp2p::PeerId::random();
        let boot: HashSet<_> = [peer].into();
        for _ in 0..MAX_DIAL_FAILURES * 2 {
            assert!(!note_dial_failure(&mut failures, &boot, peer));
        }
        assert!(failures.is_empty());
    }

    #[test]
    fn dial_failures_are_tracked_per_peer() {
        let mut failures = HashMap::new();
        let boot = HashSet::new();
        let a = libp2p::PeerId::random();
        let b = libp2p::PeerId::random();
        for _ in 0..MAX_DIAL_FAILURES - 1 {
            assert!(!note_dial_failure(&mut failures, &boot, a));
        }
        // b's single failure must not tip a over the threshold or vice versa.
        assert!(!note_dial_failure(&mut failures, &boot, b));
        assert!(note_dial_failure(&mut failures, &boot, a));
    }

    // ---------------------------* unfinalized-vote outbox *--------------------------- //

    use super::{enqueue_unfinalized, republish_outbox, MAX_OUTBOX};
    use attestor_pool::Vote;

    fn vote_at(height: u64) -> Vote {
        let sk = bls_signatures::PrivateKey::new([1u8; 32]);
        Vote {
            chain_key: 1,
            height,
            digest: attestor_primitives::Digest::from([0xAA; 32]),
            attestor: att(1),
            signature_bls: attestor_primitives::bls::WrapEncode(sk.sign([0u8; 1])),
        }
    }

    #[test]
    fn coalesced_local_production_drains_every_ready_height() {
        let mut pending = PendingVotes::new();
        pending.entry(100).or_default().insert(att(1), vote_at(100));
        pending.entry(110).or_default().insert(att(1), vote_at(110));
        pending.entry(120).or_default().insert(att(1), vote_at(120));
        pending.entry(130).or_default().insert(att(1), vote_at(130));

        // Simulate watch coalescing: p2p observes only the latest production update (120), while
        // proof data for 100 and 120 is already cached. Height 110 was skipped locally and is
        // stale; 130 is still in the future and must remain buffered.
        let mut drained =
            drain_pending_votes(&mut pending, 120, |height| matches!(height, 100 | 120));
        drained.sort_unstable_by_key(|vote| vote.height);

        assert_eq!(
            drained.iter().map(|vote| vote.height).collect::<Vec<_>>(),
            vec![100, 120]
        );
        assert_eq!(pending.keys().copied().collect::<Vec<_>>(), vec![130]);
    }

    #[test]
    fn outbox_is_bounded_and_drops_oldest_first() {
        let mut outbox = std::collections::VecDeque::new();
        for h in 0..(MAX_OUTBOX as u64 + 2) {
            enqueue_unfinalized(&mut outbox, vote_at(h));
        }
        assert_eq!(outbox.len(), MAX_OUTBOX);
        // The two oldest entries (h=0, h=1) were displaced; the newest survives at the back.
        assert_eq!(outbox.front().map(|v| v.height), Some(2));
        assert_eq!(outbox.back().map(|v| v.height), Some(MAX_OUTBOX as u64 + 1));
    }

    // The CI genesis deadlock: all three chain-4 attestors "successfully" published their genesis
    // vote once, every copy was lost to connection churn, and nothing ever re-sent — quorum could
    // never assemble. The outbox therefore must NOT treat publish success as removal; a vote only
    // leaves via the finalization prune.
    #[test]
    fn a_published_vote_stays_queued_until_finalized() {
        let mut outbox = std::collections::VecDeque::new();
        enqueue_unfinalized(&mut outbox, vote_at(0));
        enqueue_unfinalized(&mut outbox, vote_at(10));

        // Every re-gossip sweep publishes the full backlog and removes nothing.
        for _ in 0..2 {
            let published = republish_outbox(&mut outbox, |_| true);
            assert_eq!(published, 2);
            assert_eq!(outbox.len(), 2, "publish success must not dequeue");
        }

        // Only finalization prunes (mirrors the `latest_finalized_rx` arm in `run`).
        outbox.retain(|vote| vote.height > 0);
        assert_eq!(
            outbox.iter().map(|v| v.height).collect::<Vec<_>>(),
            vec![10]
        );
    }

    // Peerless sweep: stop at the first failure so an empty mesh costs one publish per tick, and
    // report zero published — nothing is lost, everything waits for the next trigger.
    #[test]
    fn republish_stops_at_the_first_failure_and_keeps_everything() {
        let mut outbox = std::collections::VecDeque::new();
        enqueue_unfinalized(&mut outbox, vote_at(0));
        enqueue_unfinalized(&mut outbox, vote_at(10));
        let mut attempts = 0;
        let published = republish_outbox(&mut outbox, |_| {
            attempts += 1;
            false
        });
        assert_eq!((published, attempts), (0, 1));
        assert_eq!(outbox.len(), 2);
    }

    // The admission window is `max(max_catchup, interval)` *blocks* above the finalized height
    // (`max_catchup` is a block-count bound, like the runtime's `MaxCatchup`).
    const INTERVAL: u64 = 30;
    const MAX_CATCHUP: u64 = 500;
    const GENESIS: u64 = 100;

    #[test]
    fn aligned_height_in_window_is_bufferable() {
        // 150 is a multiple of 30, above finalized (120), well within the window.
        assert!(is_bufferable(
            150,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(120),
            GENESIS
        ));
    }

    #[test]
    fn misaligned_height_is_rejected() {
        // 151 is neither genesis nor a multiple of the interval — production never emits there.
        assert!(!is_bufferable(
            151,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(120),
            GENESIS
        ));
    }

    #[test]
    fn genesis_height_is_allowed_even_if_not_interval_aligned() {
        // genesis (100) is not a multiple of 30 but is produced once; allow it while still
        // unfinalized.
        assert!(is_bufferable(
            GENESIS,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(90),
            GENESIS
        ));
    }

    #[test]
    fn height_at_or_below_finalized_is_rejected() {
        // Equal to finalized — already attested, nothing to wait for.
        assert!(!is_bufferable(
            120,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(120),
            GENESIS
        ));
        // Below finalized.
        assert!(!is_bufferable(
            90,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(120),
            GENESIS
        ));
    }

    #[test]
    fn window_edge_is_inclusive_but_beyond_is_rejected() {
        // 120 + 500 = 620 is not interval-aligned; the highest aligned height inside the window
        // is 600. One interval later (630) is out of the window.
        assert!(is_bufferable(
            600,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(120),
            GENESIS
        ));
        assert!(!is_bufferable(
            630,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            Some(120),
            GENESIS
        ));
    }

    #[test]
    fn next_target_stays_admissible_when_max_catchup_is_below_interval() {
        // With max_catchup < interval, the window widens to one interval so the next aligned
        // target (150) is still bufferable.
        assert!(is_bufferable(
            150,
            GENESIS,
            INTERVAL,
            10,
            Some(120),
            GENESIS
        ));
        assert!(!is_bufferable(
            180,
            GENESIS,
            INTERVAL,
            10,
            Some(120),
            GENESIS
        ));
    }

    /// Cold-start bootstrap: nothing attested on-chain yet (`finalized = None`) and
    /// `start_height == genesis`. A peer's genesis vote arriving before our own genesis data is
    /// built MUST be bufferable — production gossips it exactly once and a gossipsub `Ignore`
    /// is terminal, so dropping it here can permanently starve genesis quorum on
    /// staggered/parallel cold starts.
    #[test]
    fn genesis_vote_is_bufferable_before_anything_finalized() {
        assert!(is_bufferable(
            GENESIS,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            None,
            GENESIS
        ));
        // Later aligned heights within the window are admissible too...
        assert!(is_bufferable(
            150,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            None,
            GENESIS
        ));
        // ...but below the start floor or beyond the window stays rejected.
        assert!(!is_bufferable(
            60,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            None,
            GENESIS
        ));
        assert!(!is_bufferable(
            630,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            None,
            GENESIS
        ));
    }

    /// Restart mid-chain before the first `BlockAttested` observation: `finalized` is still
    /// `None` but `start_height` reflects the resume point — the floor is inclusive there as
    /// well (production may re-produce at exactly `start_height`).
    #[test]
    fn resume_floor_is_inclusive_while_unobserved() {
        assert!(is_bufferable(
            300,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            None,
            300
        ));
        assert!(!is_bufferable(
            270,
            GENESIS,
            INTERVAL,
            MAX_CATCHUP,
            None,
            300
        ));
    }
}
