//! libp2p gossip task.
//!
//! Differences from v1:
//!
//! - Gossips **lightweight votes** (`attestor_pool::Vote`), not full attestations.
//! - Receives outgoing votes via an `mpsc::Receiver<Vote>`, not a broadcast channel
//!   (we have one consumer, not many).
//! - Verifies incoming votes via the local `proof_cache` (peers must have produced their own
//!   AttestationData at that height to verify a remote BLS signature).

pub mod behavior;
pub mod protocols;

use std::sync::Arc;
use std::time::Instant;

use parity_scale_codec::{DecodeAll, Encode};
use tokio::sync::mpsc;

use attestor_pool::Vote;
use attestor_primitives::AttestorId;
use write_ability::envelope::{MessageVote, ReobservationRequest, SetUpdateVote};
use write_ability::protocol::{
    attestor_set_update_topic, message_votes_topic, reobservation_topic,
};

use crate::error::Error;
use crate::shared::Shared;
use crate::tasks::write_ability::ingest;
use crate::vote::{verify_vote, VerifyResult};

/// Consecutive failed pings on a single connection before we reap it.
const MAX_PING_FAILURES: u32 = 3;

/// Consecutive failed *dials* to a peer before we drop it from the Kademlia routing table.
/// Kademlia never evicts on dial failure by itself, and peers re-share stale records with each
/// other, so a node that shut down while still registered on-chain would otherwise be redialed
/// forever (the deny-list only covers attestors that were chilled/kicked). Eviction is safe: if
/// the peer comes back it re-announces via identify/kad discovery and is re-added as usual.
const MAX_DIAL_FAILURES: u32 = 5;

/// Upper bound on locally-produced votes retained for publish retry (see `retry_queue` in
/// [`run`]). Overflow drops the *oldest* entry — the height most likely to finalize without our
/// broadcast anyway.
const MAX_RETRY_QUEUE: usize = 256;

/// How often queued unpublished votes are retried while the queue is non-empty. Retries also
/// fire immediately when gossip publishing first becomes possible again (mesh regained).
const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(builder::Builder)]
pub struct Config {
    pub boot_nodes: Vec<libp2p::Multiaddr>,
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
    mut peer_deactivated_rx: mpsc::UnboundedReceiver<AttestorId>,
    mut mv_publish_rx: mpsc::Receiver<MessageVote>,
    mut set_update_publish_rx: mpsc::Receiver<SetUpdateVote>,
) -> Result<(), Error> {
    use futures::StreamExt as _;

    let enable_mdns = !cfg.no_mdns;
    let chain_key = cfg.chain_key;

    // Built before the swarm because the behavior needs the topic hash to install its
    // gossipsub peer-scoring topic parameters.
    let topic = libp2p::gossipsub::IdentTopic::new(format!("{chain_key}/attest"));

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
        .with_behaviour(|k| behavior::P2PBehavior::new(k, enable_mdns, &topic))
        .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?
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
    let mv_topic = shared.message_votes.is_some().then(|| {
        let t = libp2p::gossipsub::IdentTopic::new(message_votes_topic(chain_key));
        tracing::info!(topic = %t, "📫 subscribing to message-vote gossip");
        t
    });
    if let Some(mv_topic) = &mv_topic {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(mv_topic)
            .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;
    }

    // Reobservation requests (liveness recovery) ride the same swarm on their own topic. We only
    // *receive* these (relayers publish them); on a valid request we re-verify + re-sign. Subscribed
    // alongside the vote topic so message attestation is fully enabled or fully off.
    let reobs_topic = shared.message_votes.is_some().then(|| {
        let t = libp2p::gossipsub::IdentTopic::new(reobservation_topic(chain_key));
        tracing::info!(topic = %t, "📫 subscribing to reobservation requests");
        t
    });
    if let Some(reobs_topic) = &reobs_topic {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(reobs_topic)
            .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;
    }

    // Attestor-set-update votes (P2-8) ride the same swarm on their own topic. We only *publish*
    // these (the proposer produces them; the relayer consumes them). Subscribing is required to
    // publish/propagate. Gated on message attestation like the topics above.
    let set_update_topic = shared.message_votes.is_some().then(|| {
        let t = libp2p::gossipsub::IdentTopic::new(attestor_set_update_topic(chain_key));
        tracing::info!(topic = %t, "📫 subscribing to attestor-set-update gossip");
        t
    });
    if let Some(set_update_topic) = &set_update_topic {
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(set_update_topic)
            .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;
    }

    for address in cfg.boot_nodes {
        let Some(peer_id) = address.iter().find_map(|p| match p {
            libp2p::multiaddr::Protocol::P2p(pid) => Some(pid),
            _ => None,
        }) else {
            tracing::error!(%address, "missing peer id in multiaddr");
            continue;
        };
        boot_peers.insert(peer_id);
        swarm.behaviour_mut().kad.add_address(&peer_id, address);
    }

    // Fail fast on a mesh-less configuration. There is no explicit `swarm.dial()` anywhere —
    // connectivity is entirely kad-bootstrap-driven off the boot addresses (plus mdns locally) —
    // so with mdns disabled (the k8s reality) and no usable bootnode the node sits permanently
    // peerless while reporting healthy: /health has no p2p input and the task never exits.
    // Malformed multiaddrs (missing /p2p/<peer-id>) are skipped above with only an error log, so
    // an all-malformed list would otherwise fail just as silently as an empty one.
    //
    // EXCEPT a node that advertises a `public_addr` is a *seed* (the mesh bootnode): peers dial
    // IT, so it forms a mesh without any boot peers of its own. Requiring it to have a boot node
    // would be circular — the bootnode is the root of the mesh. Only fail-fast for a node that is
    // neither seeded (boot peers / mdns) nor itself discoverable as a seed (public_addr).
    if boot_peers.is_empty() && !enable_mdns && cfg.public_addr.is_none() {
        return Err(Error::P2p(anyhow::anyhow!(
            "no usable boot nodes (list empty or every multiaddr missing its /p2p/<peer-id> \
             suffix), mdns is disabled, and no public_addr is set — this node can neither \
             discover a peer nor be discovered as a seed"
        )));
    }

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

    // Known attestor → libp2p peer id bindings, learned from the (gossipsub-signed) authorship of
    // BLS-verified votes. `message.source` is the original signer and survives relaying, so it
    // reliably identifies an attestor's own peer even when the relaying neighbour differs. Used to
    // (a) evict a chilled/kicked attestor's peer on the production nudge and (b) refuse to re-add
    // or reconnect that peer while it stays chilled (the deny-list). A `Vec` suffices: the set is
    // bounded by committee size and only ever linearly scanned. The p2p keypair is ed25519 while
    // the on-chain attestor id is sr25519, so this binding cannot be derived — it must be learned.
    let mut peers_by_attestor: Vec<(AttestorId, libp2p::PeerId)> = Vec::new();

    // Per-peer + global rate limit for inbound reobservation requests (audit P1-4).
    let mut reobs_admission = ReobsAdmission::new(Instant::now());

    let mut local_produced_rx = shared.local_produced_rx.clone();
    let mut latest_finalized_rx = shared.latest_finalized_rx.clone();

    // Locally-produced votes that could not be published (no mesh peers yet, or a transient
    // publish failure). Retried when the mesh comes back and on a fixed interval; entries at or
    // below the finalized height are pruned since they no longer need propagation. Without this,
    // votes produced while peerless were silently lost: the channel backed up until production
    // dropped fresh broadcasts, and publish failures discarded the vote outright.
    let mut retry_queue: std::collections::VecDeque<Vote> = std::collections::VecDeque::new();
    // Same, for locally signed message votes: a vote produced while the message-votes mesh has no
    // peers must not be lost — the relayer can only count votes it hears. Bounded like
    // `retry_queue`; message votes need no finalized-height pruning (stale entries age out of the
    // relayer's aggregation window harmlessly, and the cap evicts the oldest first).
    let mut mv_retry_queue: std::collections::VecDeque<MessageVote> =
        std::collections::VecDeque::new();
    let mut retry_tick = tokio::time::interval(RETRY_INTERVAL);
    retry_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            biased;
            _ = shared.token.cancelled() => return Ok(()),

            // Outgoing — production gives us a freshly built local vote to gossip. Always drain
            // the channel (even while peerless): the channel is unbounded, so draining
            // unconditionally is what keeps it from growing; anything unpublishable goes to the
            // bounded retry queue. Always *attempt* the publish — `try_publish` failing is the
            // authoritative no-peers signal; pre-gating on the mesh hint wedged publishing
            // whenever the hint went stale-false (see `can_broadcast`).
            Some(vote) = gossip_rx.recv() => {
                if !try_publish(&mut swarm, &topic, &vote) {
                    queue_for_retry(&mut retry_queue, vote);
                }
            }

            // Periodic retry of unpublished votes (block attestation + message votes).
            // Unconditional attempts (no mesh-hint gate — see `can_broadcast`); each flush stops
            // at its first failure, so a peerless tick costs one publish call per queue per 30s.
            _ = retry_tick.tick(), if !retry_queue.is_empty() || !mv_retry_queue.is_empty() => {
                flush_retry_queue(&mut swarm, &topic, &mut retry_queue);
                if let Some(mv_topic) = &mv_topic {
                    flush_mv_retry_queue(&mut swarm, mv_topic, &mut mv_retry_queue);
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
                    retry_queue.retain(|vote| vote.height > fin);
                }
            }

            // Production observed a chill/kick on chain and refreshed the active set — evict the
            // deactivated attestor's peer (if we know it) and keep it out until it reactivates.
            Some(attestor) = peer_deactivated_rx.recv() => {
                handle_peer_deactivated(&shared, &mut swarm, &peers_by_attestor, &attestor);
            }

            // Outgoing — the write_ability task produced a signed message vote to gossip. A
            // publish that fails (typically no mesh peers yet) is queued and retried, same as
            // block-attestation votes: dropping it would silently cost the relayer our signature.
            Some(vote) = mv_publish_rx.recv(), if mv_topic.is_some() => {
                if let Some(mv_topic) = &mv_topic {
                    if !try_publish_message_vote(&mut swarm, mv_topic, &vote) {
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
                if !could_broadcast && can_broadcast && !retry_queue.is_empty() {
                    flush_retry_queue(&mut swarm, &topic, &mut retry_queue);
                }
                if !could_broadcast_mv && can_broadcast_mv {
                    if let Some(mv_topic) = &mv_topic {
                        flush_mv_retry_queue(&mut swarm, mv_topic, &mut mv_retry_queue);
                    }
                }
            }
        }
    }
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

    fn try_take(&mut self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Global + per-peer reobservation admission. `admit` charges the per-peer bucket first (one peer
/// can't drain the global allowance) then the global bucket; both must have a token.
struct ReobsAdmission {
    global: TokenBucket,
    per_peer: std::collections::HashMap<libp2p::PeerId, TokenBucket>,
    order: std::collections::VecDeque<libp2p::PeerId>,
}

impl ReobsAdmission {
    fn new(now: Instant) -> Self {
        Self {
            global: TokenBucket::new(REOBS_GLOBAL_CAPACITY, REOBS_GLOBAL_REFILL_PER_SEC, now),
            per_peer: std::collections::HashMap::new(),
            order: std::collections::VecDeque::new(),
        }
    }

    fn admit(&mut self, source: libp2p::PeerId, now: Instant) -> bool {
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
        // unwrap: just ensured the entry exists.
        if !self.per_peer.get_mut(&source).unwrap().try_take(now) {
            return false;
        }
        self.global.try_take(now)
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
                // votes (the relayer does). Accept inbound frames so gossipsub keeps relaying, but
                // take no action. Crucially, do not fall through to the block-attestation `Vote`
                // decoder below: a SetUpdateVote SCALE payload would fail to decode and be Rejected,
                // applying gossipsub penalties to peers relaying legitimate set-update votes (bugbot).
                libp2p::gossipsub::MessageAcceptance::Accept
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

                // Learn the attestor → peer id binding from the *original signer* of a BLS-verified
                // vote (`message.source`, preserved across relays), not the relaying neighbour. Only
                // recorded when the vote cryptographically verified as the attestor's, so the binding
                // is trustworthy. This is what later lets us evict / deny that peer once its attestor
                // is chilled.
                if let (Some(attestor), Some(source)) = (learned, message.source) {
                    note_attestor_peer(peers_by_attestor, attestor, source);
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
            swarm
                .behaviour_mut()
                .gossipsub
                .report_message_validation_result(&message_id, &propagation_source, decision);
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
fn try_publish(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    vote: &Vote,
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
        Err(err) => {
            tracing::warn!(
                digest = ?vote.digest,
                height = vote.height,
                %err,
                "✉️ gossip publish failed — queueing for retry",
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
    vote: &MessageVote,
) -> bool {
    match swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.hash(), vote.encode_bytes())
    {
        Ok(_) => {
            tracing::info!(chain_key = vote.chain_key, "✉️ gossiped message vote");
            true
        }
        Err(libp2p::gossipsub::PublishError::Duplicate) => true,
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
    mv_retry_queue: &mut std::collections::VecDeque<MessageVote>,
    vote: MessageVote,
) {
    if mv_retry_queue.len() >= MAX_RETRY_QUEUE {
        if let Some(dropped) = mv_retry_queue.pop_front() {
            tracing::warn!(
                chain_key = dropped.chain_key,
                cap = MAX_RETRY_QUEUE,
                "🗑️ message-vote retry queue full — dropping oldest unpublished vote"
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
    mv_retry_queue: &mut std::collections::VecDeque<MessageVote>,
) {
    let backlog = mv_retry_queue.len();
    while let Some(vote) = mv_retry_queue.front() {
        if try_publish_message_vote(swarm, topic, vote) {
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

/// Append a vote to the bounded retry queue, dropping the oldest entry on overflow (lowest
/// height — the one most likely to finalize without our broadcast).
fn queue_for_retry(retry_queue: &mut std::collections::VecDeque<Vote>, vote: Vote) {
    if retry_queue.len() >= MAX_RETRY_QUEUE {
        if let Some(dropped) = retry_queue.pop_front() {
            tracing::warn!(
                digest = ?dropped.digest,
                height = dropped.height,
                cap = MAX_RETRY_QUEUE,
                "🗑️ retry queue full — dropping oldest unpublished vote"
            );
        }
    }
    retry_queue.push_back(vote);
}

/// Republish queued votes in order (oldest first) until one fails, which usually means the mesh
/// went away again — the remainder stays queued for the next trigger.
fn flush_retry_queue(
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    retry_queue: &mut std::collections::VecDeque<Vote>,
) {
    let backlog = retry_queue.len();
    while let Some(vote) = retry_queue.front() {
        if try_publish(swarm, topic, vote) {
            retry_queue.pop_front();
        } else {
            break;
        }
    }
    if retry_queue.len() < backlog {
        tracing::info!(
            published = backlog - retry_queue.len(),
            remaining = retry_queue.len(),
            "📤 flushed unpublished vote backlog"
        );
    }
}

// -------------------------------------* peer deny-list *-------------------------------------- //

/// Records (or refreshes) the libp2p peer id we've observed for an attestor. Called only for
/// BLS-verified votes, so the binding is trustworthy. A single attestor keeps at most one peer id;
/// a node that re-registers under a *new* attestor id simply adds a second `(attestor, peer)`
/// entry, which is exactly what lets [`deny_decision`] treat a peer as active while *any* attestor
/// mapped to it is active.
fn note_attestor_peer(
    peers_by_attestor: &mut Vec<(AttestorId, libp2p::PeerId)>,
    attestor: AttestorId,
    peer_id: libp2p::PeerId,
) {
    // `position` (shared borrow) then index (mutable borrow) keeps the two borrows disjoint, so
    // the `push` in the not-found case is accepted without relying on Polonius.
    if let Some(pos) = peers_by_attestor.iter().position(|(a, _)| *a == attestor) {
        let existing = &mut peers_by_attestor[pos].1;
        if *existing != peer_id {
            tracing::info!(%attestor, old = %existing, new = %peer_id, "🔁 attestor peer id changed");
            *existing = peer_id;
        }
    } else {
        tracing::debug!(%attestor, %peer_id, "🪪 learned attestor peer id");
        peers_by_attestor.push((attestor, peer_id));
    }
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
/// Takes the [`PeerId`] by value because `disconnect_peer_id` consumes it and `PeerId` is not
/// `Copy`.
///
/// [`PeerId`]: libp2p::PeerId
fn evict_peer(swarm: &mut libp2p::Swarm<behavior::P2PBehavior>, peer_id: libp2p::PeerId) {
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
    shared: &Arc<Shared>,
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    peers_by_attestor: &[(AttestorId, libp2p::PeerId)],
    attestor: &AttestorId,
) {
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
        if !is_peer_denied(shared, peers_by_attestor, &peer_id) {
            tracing::debug!(%attestor, %peer_id, "↩️  peer still maps to an active attestor — keeping");
            continue;
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
            message_hash,
        } => {
            shared.metrics.note_message_vote();
            if reached_threshold {
                ingest::note_threshold(shared.chain_key, &message_hash);
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
/// swarm loop must stay responsive, so we only decode + forward here (no RPC / signing). We Accept
/// any well-formed request so it keeps propagating to other attestors — each re-verifies it
/// independently — even if our own forward buffer is momentarily full.
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
    // Rate-limit before doing (or propagating) any work: an over-limit reobservation flood is
    // dropped with `Ignore` so it is not amplified across the mesh (audit P1-4).
    if !admission.admit(source, Instant::now()) {
        tracing::debug!(%source, "🚦 reobservation rate limit exceeded — dropping, not propagating");
        return MessageAcceptance::Ignore;
    }
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
    match state.reobs_tx.try_send(request) {
        Ok(()) => MessageAcceptance::Accept,
        Err(mpsc::error::TrySendError::Full(_)) => {
            tracing::warn!("reobservation queue full — dropping locally but still propagating");
            MessageAcceptance::Accept
        }
        Err(mpsc::error::TrySendError::Closed(_)) => MessageAcceptance::Ignore,
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
    use super::{
        deny_decision, drain_pending_votes, is_bufferable, PendingVotes, ReobsAdmission,
        REOBS_PER_PEER_CAPACITY,
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
            assert!(a.admit(p, t0), "request {i} within per-peer capacity");
        }
        assert!(!a.admit(p, t0), "over per-peer capacity");
        // …a different peer has its own bucket (shares only the global allowance)…
        let q = libp2p::PeerId::random();
        assert!(a.admit(q, t0));
        // …and the per-peer bucket refills over time (1 token/sec).
        assert!(a.admit(p, t0 + Duration::from_secs(1)));
    }

    fn att(n: u8) -> AttestorId {
        AttestorId::from_public([n; 32])
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

    // ---------------------------* publish retry queue *--------------------------- //

    use super::{queue_for_retry, MAX_RETRY_QUEUE};
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
    fn retry_queue_is_bounded_and_drops_oldest_first() {
        let mut queue = std::collections::VecDeque::new();
        for h in 0..(MAX_RETRY_QUEUE as u64 + 2) {
            queue_for_retry(&mut queue, vote_at(h));
        }
        assert_eq!(queue.len(), MAX_RETRY_QUEUE);
        // The two oldest entries (h=0, h=1) were displaced; the newest survives at the back.
        assert_eq!(queue.front().map(|v| v.height), Some(2));
        assert_eq!(
            queue.back().map(|v| v.height),
            Some(MAX_RETRY_QUEUE as u64 + 1)
        );
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
