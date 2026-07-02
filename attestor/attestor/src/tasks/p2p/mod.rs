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

use parity_scale_codec::{Decode, Encode};
use tokio::sync::mpsc;

use attestor_pool::Vote;
use attestor_primitives::AttestorId;

use crate::error::Error;
use crate::shared::Shared;
use crate::vote::{verify_vote, VerifyResult};

/// Consecutive failed pings on a single connection before we reap it.
const MAX_PING_FAILURES: u32 = 3;

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
    mut gossip_rx: mpsc::Receiver<Vote>,
    mut peer_deactivated_rx: mpsc::UnboundedReceiver<AttestorId>,
) -> Result<(), Error> {
    use futures::StreamExt as _;

    let enable_mdns = !cfg.no_mdns;
    let chain_key = cfg.chain_key;

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
        .with_behaviour(|k| behavior::P2PBehavior::new(k, enable_mdns))
        .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?
        .build();

    let topic = libp2p::gossipsub::IdentTopic::new(format!("{chain_key}/attest"));
    tracing::info!(%topic, "📫 subscribing to lightweight attestation gossip");
    swarm
        .behaviour_mut()
        .gossipsub
        .subscribe(&topic)
        .map_err(|e| Error::P2p(anyhow::anyhow!("{e}")))?;

    for address in cfg.boot_nodes {
        let Some(peer_id) = address.iter().find_map(|p| match p {
            libp2p::multiaddr::Protocol::P2p(pid) => Some(pid),
            _ => None,
        }) else {
            tracing::error!(%address, "missing peer id in multiaddr");
            continue;
        };
        swarm.behaviour_mut().kad.add_address(&peer_id, address);
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

    let mut can_broadcast = false;

    // Per-height pending buffer for incoming votes that arrived before our local production
    // reached that height. Drained when `shared.local_produced_rx` changes. Bounded per height
    // to avoid memory bloat from spammy peers; spillover is dropped (gossipsub heartbeats
    // will retransmit anyway).
    const MAX_PENDING_PER_HEIGHT: usize = 32;
    let mut pending_votes: std::collections::HashMap<attestor_primitives::Height, Vec<Vote>> =
        std::collections::HashMap::new();

    // Modern libp2p ping no longer tears down a connection on repeated failures; we do it
    // ourselves. Tracks consecutive failed pings per connection and reaps the connection once it
    // crosses MAX_PING_FAILURES so a wedged socket can't silently starve the mesh.
    let mut ping_failures: std::collections::HashMap<libp2p::swarm::ConnectionId, u32> =
        std::collections::HashMap::new();

    // Known attestor → libp2p peer id bindings, learned from the (gossipsub-signed) authorship of
    // BLS-verified votes. `message.source` is the original signer and survives relaying, so it
    // reliably identifies an attestor's own peer even when the relaying neighbour differs. Used to
    // (a) evict a chilled/kicked attestor's peer on the production nudge and (b) refuse to re-add
    // or reconnect that peer while it stays chilled (the deny-list). A `Vec` suffices: the set is
    // bounded by committee size and only ever linearly scanned. The p2p keypair is ed25519 while
    // the on-chain attestor id is sr25519, so this binding cannot be derived — it must be learned.
    let mut peers_by_attestor: Vec<(AttestorId, libp2p::PeerId)> = Vec::new();

    let mut local_produced_rx = shared.local_produced_rx.clone();
    let mut latest_finalized_rx = shared.latest_finalized_rx.clone();

    loop {
        tokio::select! {
            biased;
            _ = shared.token.cancelled() => return Ok(()),

            // Outgoing — production gives us a freshly built local vote to gossip.
            Some(vote) = gossip_rx.recv(), if can_broadcast => {
                let bytes = vote.encode();
                if let Err(err) = swarm.behaviour_mut().gossipsub.publish(topic.hash(), bytes) {
                    tracing::warn!(
                        digest = ?vote.digest,
                        height = vote.height,
                        %err,
                        "✉️ gossip publish failed",
                    );
                } else {
                    tracing::info!(
                        digest = ?vote.digest,
                        height = vote.height,
                        attestor = %vote.attestor,
                        "✉️ gossiped vote",
                    );
                }
            }

            // Local production cached new AttestationData → drain any votes we held back at this
            // height, then bulk-prune everything at or below it. We produce each height at most
            // once, so lower entries (including off-schedule heights a peer may have injected)
            // can never become verifiable and would otherwise linger forever.
            res = local_produced_rx.changed() => {
                if res.is_err() { return Ok(()); }
                let Some(h) = *local_produced_rx.borrow() else { continue; };
                if let Some(queued) = pending_votes.remove(&h) {
                    for vote in queued {
                        let _ = retry_pending_vote(&shared, vote).await;
                    }
                }
                pending_votes.retain(|&height, _| height > h);
            }

            // An attestation finalized on chain → drop every buffered vote at or below it. Bounds
            // the buffer even when our local production schedule never reaches the heights a peer
            // buffered (e.g. it stalled, or the votes were just shy of producible).
            res = latest_finalized_rx.changed() => {
                if res.is_err() { return Ok(()); }
                let finalized = latest_finalized_rx.borrow().map(|info| info.height);
                if let Some(fin) = finalized {
                    pending_votes.retain(|&height, _| height > fin);
                }
            }

            // Production observed a chill/kick on chain and refreshed the active set — evict the
            // deactivated attestor's peer (if we know it) and keep it out until it reactivates.
            Some(attestor) = peer_deactivated_rx.recv() => {
                handle_peer_deactivated(&shared, &mut swarm, &peers_by_attestor, &attestor);
            }

            // Incoming events from the swarm.
            event = swarm.select_next_some() => {
                handle_swarm(
                    &shared,
                    &mut swarm,
                    &topic,
                    &mut can_broadcast,
                    &mut pending_votes,
                    MAX_PENDING_PER_HEIGHT,
                    &mut ping_failures,
                    &mut peers_by_attestor,
                    event,
                ).await;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_swarm(
    shared: &Arc<Shared>,
    swarm: &mut libp2p::Swarm<behavior::P2PBehavior>,
    topic: &libp2p::gossipsub::IdentTopic,
    can_broadcast: &mut bool,
    pending_votes: &mut std::collections::HashMap<attestor_primitives::Height, Vec<Vote>>,
    max_pending_per_height: usize,
    ping_failures: &mut std::collections::HashMap<libp2p::swarm::ConnectionId, u32>,
    peers_by_attestor: &mut Vec<(AttestorId, libp2p::PeerId)>,
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

            let (acceptance, learned) =
                handle_vote_msg(shared, pending_votes, max_pending_per_height, &message.data).await;

            // Learn the attestor → peer id binding from the *original signer* of a BLS-verified
            // vote (`message.source`, preserved across relays), not the relaying neighbour. Only
            // recorded when the vote cryptographically verified as the attestor's, so the binding
            // is trustworthy. This is what later lets us evict / deny that peer once its attestor
            // is chilled.
            if let (Some(attestor), Some(source)) = (learned, message.source) {
                note_attestor_peer(peers_by_attestor, attestor, source);
            }

            let decision = match acceptance {
                Acceptance::Accept => libp2p::gossipsub::MessageAcceptance::Accept,
                Acceptance::Ignore => libp2p::gossipsub::MessageAcceptance::Ignore,
                Acceptance::Reject => {
                    shared.metrics.increase_invalid_gossipsub_count();
                    libp2p::gossipsub::MessageAcceptance::Reject
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
            }
            ping_failures.remove(&connection_id);
            *can_broadcast = swarm
                .behaviour()
                .gossipsub
                .mesh_peers(&topic.hash())
                .next()
                .is_some();
        }
        SwarmEvent::Behaviour(P2PBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Subscribed { .. },
        ))
        | SwarmEvent::Behaviour(P2PBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Unsubscribed { .. },
        )) => {
            *can_broadcast = swarm
                .behaviour()
                .gossipsub
                .mesh_peers(&topic.hash())
                .next()
                .is_some();
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
            }
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
            // Only drop a peer for unambiguously malicious / unrecoverable errors. v1 logic
            // verbatim, condensed: WrongPeerId and Denied → remove; everything else → log.
            match error {
                libp2p::swarm::DialError::WrongPeerId { .. }
                | libp2p::swarm::DialError::Denied { .. } => {
                    if let Some(p) = peer_id {
                        swarm.behaviour_mut().kad.remove_peer(&p);
                    }
                }
                _ => {}
            }
        }
        _ => {}
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

async fn handle_vote_msg(
    shared: &Arc<Shared>,
    pending_votes: &mut std::collections::HashMap<attestor_primitives::Height, Vec<Vote>>,
    max_pending_per_height: usize,
    bytes: &[u8],
) -> (Acceptance, Option<AttestorId>) {
    let mut slice = bytes;
    let Ok(vote) = Vote::decode(&mut slice) else {
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
                    // Bad attestor / known invalid — reject.
                    PoolError::Unauthorized(..) | PoolError::KnownInvalid(..) => Acceptance::Reject,
                }
            }
            None => Acceptance::Ignore,
        },
        VerifyResult::NoLocal => {
            // We haven't produced at this height yet. Before buffering, gate on two cheap checks
            // so a peer can't grow this map without bound:
            //   * membership — we can't BLS-verify without local data, but we CAN check the sender
            //     is in the active attestor set. `verify_vote` returns `NoLocal` *before* its
            //     membership check, so do it here; an unknown sender is rejected outright rather
            //     than buffered.
            //   * producible height — the height must be one we could actually produce locally
            //     (on the attestation schedule and within the catch-up window). Off-schedule or
            //     far-future heights never gain local data, so buffering them is pure memory-
            //     attack surface; drop them (Ignore) instead.
            if pubkey.is_none() {
                tracing::warn!(
                    attestor = %vote.attestor,
                    height = vote.height,
                    "👤 unknown attestor at no-local height — rejecting"
                );
                Acceptance::Reject
            } else if !worth_buffering(shared, vote.height) {
                tracing::debug!(
                    digest = ?vote.digest,
                    height = vote.height,
                    "🚮 no-local vote outside producible window — dropping"
                );
                Acceptance::Ignore
            } else {
                // Queue the vote — when production caches local data at this height it'll signal
                // us via `local_produced_rx` and we'll drain the queue + re-verify. Bounded per
                // height; once full, Ignore so gossipsub retransmission can fill in later. Still
                // Ignore on gossip propagation (not Accept) so we don't propagate a vote we
                // haven't verified yet.
                let entry = pending_votes.entry(vote.height).or_default();
                if entry.len() < max_pending_per_height {
                    tracing::debug!(
                        digest = ?vote.digest,
                        height = vote.height,
                        queued = entry.len() + 1,
                        "🕳️ no local data yet — queuing vote"
                    );
                    entry.push(vote);
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
        }
        VerifyResult::DivergentDigest => {
            tracing::warn!(digest = ?vote.digest, height = vote.height, "↯ divergent digest from peer");
            // It might be a real fork on someone else's chain — let it propagate (Ignore = no
            // reject) so other attestors can see it and detect a network split.
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
            tracing::warn!(attestor = %vote.attestor, "👤 unknown attestor");
            Acceptance::Reject
        }
    };

    (acceptance, learned)
}

/// Whether a `NoLocal` vote at `height` is worth buffering. It must sit on the local attestation
/// schedule (so matching local data could ever exist for it) and inside the same admission window
/// the pool itself uses. Anything off-schedule or out-of-window can never become verifiable and
/// would only grow the pending buffer — the call sites drop those.
fn worth_buffering(shared: &Arc<Shared>, height: attestor_primitives::Height) -> bool {
    let interval = shared.attestation_interval().get();
    let finalized = shared
        .latest_finalized_rx
        .borrow()
        .map(|info| info.height)
        .unwrap_or(shared.start_height);
    is_bufferable(height, shared.genesis, interval, finalized)
}

/// Pure predicate behind [`worth_buffering`], split out so the schedule/window logic is unit
/// testable without a full [`Shared`].
fn is_bufferable(
    height: attestor_primitives::Height,
    genesis: attestor_primitives::Height,
    interval: attestor_primitives::Height,
    finalized: attestor_primitives::Height,
) -> bool {
    // `StreamAttestation` emits the genesis attestation once and every later attestation at an
    // absolute multiple of the interval (`next - next % interval`). A height that is neither can
    // never gain local data.
    if height != genesis && height % interval != 0 {
        return false;
    }
    // Bound to the window the pool would admit (see `ValidateQuorum::height_admissible`): strictly
    // above the last finalized attestation and within `max_catchup` intervals of it. Anchoring on
    // the finalized height (not local production, which only climbs) keeps the buffer bounded to
    // at most `max_catchup` distinct heights.
    let window = common::constants::MAX_CATCHUP
        .get()
        .saturating_mul(interval);
    height > finalized && height <= finalized.saturating_add(window)
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
    use super::{deny_decision, is_bufferable};
    use attestor_primitives::AttestorId;

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

    // MAX_CATCHUP = 500, so the admission window is 500 * interval above the finalized height.
    const INTERVAL: u64 = 30;
    const GENESIS: u64 = 100;

    #[test]
    fn aligned_height_in_window_is_bufferable() {
        // 150 is a multiple of 30, above finalized (120), well within the window.
        assert!(is_bufferable(150, GENESIS, INTERVAL, 120));
    }

    #[test]
    fn misaligned_height_is_rejected() {
        // 151 is neither genesis nor a multiple of the interval — production never emits there.
        assert!(!is_bufferable(151, GENESIS, INTERVAL, 120));
    }

    #[test]
    fn genesis_height_is_allowed_even_if_not_interval_aligned() {
        // genesis (100) is not a multiple of 30 but is produced once; allow it while still
        // unfinalized.
        assert!(is_bufferable(GENESIS, GENESIS, INTERVAL, 90));
    }

    #[test]
    fn height_at_or_below_finalized_is_rejected() {
        // Equal to finalized — already attested, nothing to wait for.
        assert!(!is_bufferable(120, GENESIS, INTERVAL, 120));
        // Below finalized.
        assert!(!is_bufferable(90, GENESIS, INTERVAL, 120));
    }

    #[test]
    fn window_edge_is_inclusive_but_beyond_is_rejected() {
        let window = 500 * INTERVAL;
        // Exactly at finalized + window (and interval-aligned) is admitted.
        assert!(is_bufferable(120 + window, GENESIS, INTERVAL, 120));
        // One interval past the window is dropped.
        assert!(!is_bufferable(
            120 + window + INTERVAL,
            GENESIS,
            INTERVAL,
            120
        ));
    }
}
