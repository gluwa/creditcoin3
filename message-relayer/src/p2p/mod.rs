//! Read-only libp2p subscriber for attestor message-vote gossip.
//!
//! Builds one swarm shared across all configured routes (the libp2p mesh is one network — only
//! the topics differ by chain_key). For each gossipsub `Message` event the worker decodes a
//! [`MessageVote`] and forwards it to the vote pool over `vote_tx`. Validation of the vote
//! contents (signer in allowlist, ecrecover, dedup) lives in the pool — keeping the libp2p
//! task small means a single bad peer cannot stall vote routing for everyone else.
//!
//! The relayer does not publish on these topics — it is a passive observer (PoC §1).

use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use bip39::Mnemonic;
use libp2p::futures::StreamExt;
use libp2p::gossipsub::{IdentTopic, MessageAcceptance, TopicHash};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, trace, warn};
use zeroize::Zeroize;

use crate::config::P2pConfig;
use crate::prom::Metrics;

pub mod behavior;
pub mod envelope;
pub mod protocols;

pub use behavior::RelayerBehavior;
pub use envelope::MessageVote;

/// Backoff between retries when the swarm fails to start. Picked to recover quickly from
/// transient port conflicts without spamming the log.
const SWARM_RESTART_BACKOFF: Duration = Duration::from_secs(5);

/// Spawn the libp2p worker. Returns when `cancel` fires or the swarm task exits.
pub async fn run(
    p2p: P2pConfig,
    chain_keys: Vec<u64>,
    vote_tx: mpsc::Sender<MessageVote>,
    metrics: Metrics,
    cancel: CancellationToken,
) -> Result<()> {
    let keypair =
        derive_keypair(p2p.identity.as_deref()).context("failed to derive libp2p identity")?;
    let local_peer_id = keypair.public().to_peer_id();
    info!(%local_peer_id, "🛜 relayer libp2p identity ready");

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|key| RelayerBehavior::new(key, !p2p.no_mdns))?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // Subscribe to one topic per route — same mesh, different topic-id per chain_key.
    let mut topic_to_chain_key: HashMap<TopicHash, u64> = HashMap::new();
    for ck in &chain_keys {
        let topic = IdentTopic::new(protocols::message_votes_topic(*ck));
        info!(chain_key = ck, topic = %topic, "📥 subscribing to attestor votes");
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&topic)
            .with_context(|| format!("subscribe to {topic} failed"))?;
        topic_to_chain_key.insert(topic.hash(), *ck);
    }

    // Boot nodes (best-effort: parse what we can, log what we cannot).
    for boot in &p2p.boot_nodes {
        match boot.parse::<libp2p::Multiaddr>() {
            Ok(addr) => {
                if let Some(peer_id) = addr.iter().find_map(|p| match p {
                    libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
                    _ => None,
                }) {
                    info!(%addr, %peer_id, "👥 registering boot node");
                    swarm.behaviour_mut().kad.add_address(&peer_id, addr);
                } else {
                    warn!(%addr, "boot node address has no /p2p/ component; skipping");
                }
            }
            Err(err) => warn!(%boot, %err, "could not parse boot node multiaddr; skipping"),
        }
    }

    if let Some(public) = &p2p.public_addr {
        match format!("/dns4/{public}/tcp/{}", p2p.port).parse::<libp2p::Multiaddr>() {
            Ok(addr) => {
                info!(%addr, "📰 broadcasting external address");
                swarm.add_external_address(addr);
            }
            Err(err) => warn!(public, port = p2p.port, %err, "invalid public_addr"),
        }
    }

    let listen: libp2p::Multiaddr = format!("/ip4/0.0.0.0/tcp/{}", p2p.port).parse()?;
    if let Err(err) = swarm.listen_on(listen.clone()) {
        warn!(%listen, %err, "swarm listen failed; will retry after backoff");
        tokio::time::sleep(SWARM_RESTART_BACKOFF).await;
    }

    info!(routes = chain_keys.len(), "✅ libp2p subscriber online");

    let mut peer_counts: HashMap<u64, usize> = HashMap::new();

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!("🛑 libp2p worker exiting on cancel");
                return Ok(());
            }
            event = swarm.select_next_some() => {
                handle_swarm_event(
                    event,
                    &mut swarm,
                    &topic_to_chain_key,
                    &vote_tx,
                    metrics.as_ref(),
                    &mut peer_counts,
                ).await;
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn handle_swarm_event(
    event: libp2p::swarm::SwarmEvent<behavior::RelayerBehaviorEvent>,
    swarm: &mut libp2p::Swarm<RelayerBehavior>,
    topic_to_chain_key: &HashMap<TopicHash, u64>,
    vote_tx: &mpsc::Sender<MessageVote>,
    metrics: &dyn crate::prom::MetricsTrait,
    peer_counts: &mut HashMap<u64, usize>,
) {
    use behavior::RelayerBehaviorEvent;
    match event {
        libp2p::swarm::SwarmEvent::Behaviour(RelayerBehaviorEvent::Identify(
            libp2p::identify::Event::Received {
                peer_id,
                info: libp2p::identify::Info { listen_addrs, .. },
                ..
            },
        )) => {
            for addr in listen_addrs {
                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
            }
        }
        libp2p::swarm::SwarmEvent::Behaviour(RelayerBehaviorEvent::Mdns(
            libp2p::mdns::Event::Discovered(peers),
        )) => {
            for (peer_id, addr) in peers {
                debug!(%peer_id, %addr, "🛰️ mDNS discovered");
                swarm.behaviour_mut().kad.add_address(&peer_id, addr);
            }
        }
        libp2p::swarm::SwarmEvent::Behaviour(RelayerBehaviorEvent::Kad(
            libp2p::kad::Event::RoutingUpdated { peer, .. },
        )) => {
            trace!(%peer, "📋 kad routing updated");
        }
        libp2p::swarm::SwarmEvent::Behaviour(RelayerBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Message {
                propagation_source,
                message_id,
                message,
            },
        )) => {
            let Some(&chain_key) = topic_to_chain_key.get(&message.topic) else {
                trace!(topic = %message.topic, "ignoring message on unsubscribed topic");
                return;
            };
            match MessageVote::decode_bytes(&message.data) {
                Ok(vote) if vote.chain_key == chain_key => {
                    if vote_tx.send(vote).await.is_err() {
                        warn!("vote pool channel closed; libp2p worker draining");
                    }
                    swarm
                        .behaviour_mut()
                        .gossipsub
                        .report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            MessageAcceptance::Accept,
                        );
                }
                Ok(vote) => {
                    warn!(
                        %propagation_source,
                        envelope_chain_key = vote.chain_key,
                        topic_chain_key = chain_key,
                        "vote envelope chain_key disagrees with topic — rejecting"
                    );
                    swarm
                        .behaviour_mut()
                        .gossipsub
                        .report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            MessageAcceptance::Reject,
                        );
                }
                Err(err) => {
                    warn!(%propagation_source, %err, "could not decode MessageVote — rejecting");
                    metrics.inc_vote(chain_key, crate::prom::VoteOutcome::Reject);
                    swarm
                        .behaviour_mut()
                        .gossipsub
                        .report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            MessageAcceptance::Reject,
                        );
                }
            }
        }
        libp2p::swarm::SwarmEvent::Behaviour(RelayerBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Subscribed { peer_id, topic },
        )) => {
            if let Some(&chain_key) = topic_to_chain_key.get(&topic) {
                let entry = peer_counts.entry(chain_key).or_default();
                *entry += 1;
                metrics.set_p2p_peer_count(chain_key, i64::try_from(*entry).unwrap_or(i64::MAX));
            }
            trace!(%peer_id, %topic, "peer subscribed");
        }
        libp2p::swarm::SwarmEvent::Behaviour(RelayerBehaviorEvent::Gossipsub(
            libp2p::gossipsub::Event::Unsubscribed { peer_id, topic },
        )) => {
            if let Some(&chain_key) = topic_to_chain_key.get(&topic) {
                let entry = peer_counts.entry(chain_key).or_default();
                *entry = entry.saturating_sub(1);
                metrics.set_p2p_peer_count(chain_key, i64::try_from(*entry).unwrap_or(i64::MAX));
            }
            trace!(%peer_id, %topic, "peer unsubscribed");
        }
        libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
            info!(%address, "🔍 new listen address");
        }
        libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
            debug!(%peer_id, "🔗 connection established");
        }
        libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, .. } => {
            debug!(%peer_id, "⛓️‍💥 connection closed");
        }
        libp2p::swarm::SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
            debug!(?peer_id, %error, "outgoing connection error");
        }
        _ => {}
    }
}

/// Build a libp2p ed25519 keypair from the configured identity. If `identity` is `None`,
/// a fresh ephemeral key is generated and we log a warning so operators understand peers
/// will see a different `PeerId` each restart.
fn derive_keypair(identity: Option<&str>) -> Result<libp2p::identity::Keypair> {
    match identity {
        None => {
            warn!(
                "no p2p.identity configured — using an ephemeral key (PeerId changes on restart)"
            );
            Ok(libp2p::identity::Keypair::generate_ed25519())
        }
        Some(s) if s.starts_with("0x") => {
            let hex = s.strip_prefix("0x").unwrap();
            anyhow::ensure!(
                hex.len() == 64,
                "p2p.identity hex seed must be exactly 64 hex chars (32 bytes)"
            );
            let mut seed = [0u8; 32];
            hex::decode_to_slice(hex, &mut seed).context("invalid hex in p2p.identity")?;
            let kp = libp2p::identity::Keypair::ed25519_from_bytes(seed)
                .context("could not build ed25519 keypair from seed")?;
            seed.zeroize();
            Ok(kp)
        }
        Some(s) => {
            let mnemonic = Mnemonic::parse(s.trim()).context("invalid BIP39 mnemonic")?;
            let full_seed = mnemonic.to_seed_normalized("");
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&full_seed[..32]);
            let kp = libp2p::identity::Keypair::ed25519_from_bytes(seed)
                .context("could not build ed25519 keypair from mnemonic")?;
            seed.zeroize();
            Ok(kp)
        }
    }
}
