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
    let mut local_produced_rx = shared.local_produced_rx.clone();

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

            // Local production cached new AttestationData → drain any votes we held back.
            res = local_produced_rx.changed() => {
                if res.is_err() { return Ok(()); }
                let Some(h) = *local_produced_rx.borrow() else { continue; };
                if let Some(queued) = pending_votes.remove(&h) {
                    for vote in queued {
                        let _ = retry_pending_vote(&shared, vote).await;
                    }
                }
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
            for a in listen_addrs {
                swarm.behaviour_mut().kad.add_address(&peer_id, a);
            }
        }
        SwarmEvent::Behaviour(P2PBehaviorEvent::Mdns(libp2p::mdns::Event::Discovered(peers))) => {
            for (peer_id, address) in peers {
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

            let acceptance =
                handle_vote_msg(shared, pending_votes, max_pending_per_height, &message.data).await;
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
        SwarmEvent::ConnectionClosed { connection_id, .. } => {
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
            ..
        } => {
            tracing::info!(%peer_id, %connection_id, "🔗 connection up");
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
) -> Acceptance {
    let mut slice = bytes;
    let Ok(vote) = Vote::decode(&mut slice) else {
        tracing::warn!("⛔ failed to decode vote");
        return Acceptance::Reject;
    };

    let local = shared.proof_cache.local_data(vote.height);
    let pubkey = shared.bls_store.pubkey(vote.attestor.account_id());
    match verify_vote(&vote, shared.chain_key, local.as_ref(), pubkey.as_ref()) {
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
            // We haven't produced at this height yet. Queue the vote — when production caches
            // local data at this height it'll signal us via `local_produced_rx` and we'll drain
            // the queue + re-verify. Bounded per height; once full, return Ignore so gossipsub
            // retransmission can fill in later. Still return Ignore on gossip propagation
            // (not Accept) so we don't propagate a vote we haven't verified yet.
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
