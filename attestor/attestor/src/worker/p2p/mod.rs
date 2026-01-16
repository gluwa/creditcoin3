//! A [`Worker`] thread responsible for the dissemination and reception of new attestations.
//!
//! # Gossiping
//!
//! Attestations are gossiped between attestor nodes via a custom [gossipsub] network. Attestations
//! are scoped per source chain, so that network traffic for a given source chain does not impact
//! the performance of attestation production for another chain. Currently only Ethereum is
//! supported.
//!
//! # Rebroadcast
//!
//! To maintain chain liveness, past attestations are periodically re-sent to the p2p worker by the
//! [rebroadcast chain listener] as part of the [production worker]. These attestations are then
//! re-submitted to the network so that nodes which might have missed those attestations have
//! another chance at synchronization. This also helps with bootstrapping new nodes, as they might
//! have joined in the middle of attestation finalization.
//!
//! # Reception
//!
//! Whether is is from initial gossiping or rebroadcasting, the p2p worker is also responsible for
//! synchronizing new attestations it receives via gossipsub. On noticing a new attestation, the p2p
//! worker will try to deserialize it and, if successful, send it to the [attestation pool] for
//! further [`Quorum`] checks. Attestations which fail to be inserted into the attestation pool are
//! considered invalid and potentially malicious and are not forwarded to the rest of the network.
//!
//! # Attestation network p2p flow
//!
#![doc = include_str!("../../../../mermaid.html")]
//! <pre class="mermaid">
//! sequenceDiagram
//!     box Networks
//!         participant Gossipsub
//!     end
//!     box Thread 2
//!         participant Rebroadcast
//!         participant Production Worker
//!     end
//!     box Thread 3
//!         participant P2P Worker
//!     end
//!     box Shared
//!         participant Attestation Pool
//!     end
//!
//!     loop P2P
//!         alt New Remote Attestation
//!             P2P Worker -->> Gossipsub: Polls
//!
//!             activate Gossipsub
//!             Gossipsub -->> P2P Worker: New attestation
//!             deactivate Gossipsub
//!
//!             activate P2P Worker
//!             P2P Worker ->> Attestation Pool: Send Attestation
//!             deactivate P2P Worker
//!         else New Local Attestation
//!             P2P Worker ->> Production Worker: Polls
//!
//!             activate Production Worker
//!             Production Worker ->> P2P Worker: New Attestation
//!             deactivate Production Worker
//!
//!             activate P2P Worker
//!             P2P Worker -->> Gossipsub: Gossip
//!             deactivate P2P Worker
//!         else Rebroadcast Attestation
//!             P2P Worker ->> Rebroadcast: Polls
//!
//!             activate Rebroadcast
//!             Rebroadcast ->> P2P Worker: Rebroadcast Attestation
//!             deactivate Rebroadcast
//!
//!             activate P2P Worker
//!             P2P Worker -->> Gossipsub: Gossip
//!             deactivate P2P Worker
//!         else Peer Update
//!             P2P Worker -->> Gossipsub: Polls
//!
//!             activate Gossipsub
//!             Gossipsub -->> P2P Worker: New Peer/Old Peer
//!             deactivate Gossipsub
//!
//!             activate P2P Worker
//!             P2P Worker ->> P2P Worker: Update Peer List
//!             deactivate P2P Worker
//!         end
//!     end
//! </pre>
//!
//! [`Worker`]: crate::worker::Worker
//! [gossipsub]: libp2p::gossipsub
//! [rebroadcast chain listener]: crate::chain_listener::rebroadcast
//! [production worker]: crate::worker::production
//! [attestation pool]: crate::worker::validation::pool
//! [`Quorum`]: crate::worker::validation::pool::Quorum

mod behavior;
mod error;
mod protocols;

use crate::prelude::*;
pub use error::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(attestor_macro::Builder)]
pub struct Config {
    boot_nodes: Vec<libp2p::Multiaddr>,
    public_addr: Option<String>,
    port: u16,
    #[specify_later]
    keypair: libp2p::identity::Keypair,
    #[specify_later]
    receiver_p2p: tokio::sync::broadcast::Receiver<common::types::Attestation>,
    #[specify_later]
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,
    #[specify_later]
    chain_key: attestor_primitives::ChainKey,
    #[specify_later]
    metrics: common::types::Metrics,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerP2P {
    // P2P DATA
    swarm: libp2p::Swarm<behavior::P2PBehavior>,
    can_broadcast: bool,
    topic: libp2p::gossipsub::IdentTopic,
    listen_addr: libp2p::Multiaddr,

    // METRICS
    metrics: common::types::Metrics,

    // MESSAGE CHANNELS
    receiver_p2p: tokio::sync::broadcast::Receiver<common::types::Attestation>,
    sender_validation: crate::worker::validation::pool::AttestationPoolSender,
}

impl WorkerP2P {
    pub(crate) fn new(config: Config) -> common::types::Result<Self> {
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(config.keypair)
            .with_tokio()
            // Connection fallback.
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )?
            // Ideally we want to use quic to benefit from the in-build multiplexing and security.
            .with_quic()
            // Domain name resolution on peer addresses. We probably want to set up some stable
            // boot node addresses and it is better for those to be behind a domain than rely on a
            // static IP, as this gives us more flexibility around maintenance and administration.
            .with_dns()?
            .with_behaviour(behavior::P2PBehavior::new)?
            .build();

        let topic = libp2p::gossipsub::IdentTopic::new(format!("{}/attest", config.chain_key));
        tracing::info!(%topic, "📫 Subscribing to attestations");
        swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

        if !config.boot_nodes.is_empty() {
            tracing::info!("👥 Bootstrapping new remote peers");
            for address in config.boot_nodes {
                let Some(peer_id) = address.iter().find_map(|protocol| match protocol {
                    libp2p::multiaddr::Protocol::P2p(peer_id) => Some(peer_id),
                    _ => None,
                }) else {
                    panic!("Missing peer id in multiaddress: {address}");
                };

                tracing::info!(%address, "👥  at");
                swarm.behaviour_mut().kad.add_address(&peer_id, address);
            }
        } else {
            tracing::warn!("👥 Starting attestor with no boot nodes!");
        }

        if let Some(dns) = config.public_addr {
            let external_address = format!("/dns4/{}/tcp/{}", dns, config.port).parse()?;
            tracing::info!(%external_address, "📰 Broadcasting external address");
            swarm.add_external_address(external_address);
        }

        let listen_addr = format!("/ip4/0.0.0.0/tcp/{}", config.port).parse()?;

        Ok(Self {
            swarm,
            can_broadcast: false,
            topic,
            listen_addr,

            metrics: config.metrics,

            receiver_p2p: config.receiver_p2p,
            sender_validation: config.sender_validation,
        })
    }
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

impl super::Worker for WorkerP2P {
    #[tracing::instrument(name = "p2p_engine", skip_all)]
    async fn task(
        mut self,
        mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> common::types::Result<()> {
        use futures::StreamExt as _;

        // Tell the swarm to listen on all interfaces on the configured port.
        // Default port is 9000, which is useful for Kubernetes LoadBalancer services.
        self.swarm.listen_on(self.listen_addr.clone())?;

        loop {
            tokio::select! {
                biased;

                _ = &mut shutdown => {
                    break self.handle_event_shutdown().await;
                }
                attestation = self.receiver_p2p.recv(), if self.can_broadcast => {
                    self.handle_event_attestation(attestation).await?;
                }
                event = self.swarm.select_next_some() => {
                    self.handle_event_p2p(event).await?;
                }
            }
        }
    }
}

// ----------------------------------------- [ Events ] ---------------------------------------- //

impl WorkerP2P {
    // ------------------------------------* Production events *-----------------------------------

    async fn handle_event_attestation(
        &mut self,
        attestation: Result<common::types::Attestation, tokio::sync::broadcast::error::RecvError>,
    ) -> Result<(), Error> {
        use parity_scale_codec::Encode as _;

        if let Ok(attestation) = attestation {
            let digest = attestation.digest();
            let height = attestation.header_number();

            tracing::info!(
                %digest,
                height,
                attestor_id = %attestation.attestor,
                "✉️ Gossiping"
            );

            self.swarm
                .behaviour_mut()
                .gossipsub
                .publish(self.topic.hash(), attestation.encode())
                .map_err(|err| Error::PublishError(height, digest, err))?;
        };

        // We do not error if the channel is closed
        Ok(())
    }

    // ---------------------------------------* P2P events *---------------------------------------

    async fn handle_event_p2p(
        &mut self,
        event: libp2p::swarm::SwarmEvent<behavior::P2PBehaviorEvent>,
    ) -> Result<(), Error> {
        use parity_scale_codec::Decode as _;

        match event {
            // Discovering new remote peers with kad + identify
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Identify(
                libp2p::identify::Event::Received {
                    connection_id,
                    peer_id,
                    info: libp2p::identify::Info { listen_addrs, .. },
                },
            )) => {
                tracing::info!(connection = %connection_id, "🛰️ Dialing");
                tracing::info!("🛰️ Discovered new remote peer");
                for address in listen_addrs {
                    tracing::info!(%address, "🛰️  at");
                    self.swarm
                        .behaviour_mut()
                        .kad
                        .add_address(&peer_id, address.clone());
                }
            }

            // Discovering new local peers with mDNS
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Mdns(
                libp2p::mdns::Event::Discovered(peers),
            )) => {
                for (peer_id, address) in peers {
                    tracing::info!("🛰️ Discovered new local peer");
                    tracing::info!(%address, "🛰️  at");

                    self.swarm
                        .behaviour_mut()
                        .kad
                        .add_address(&peer_id, address.clone());
                }
            }

            // Kad routing table update. This is handled automatically be the libp2p switch, we
            // just get to react to the changes summarized in this event.
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Kad(
                libp2p::kad::Event::RoutingUpdated {
                    is_new_peer,
                    addresses,
                    old_peer,
                    ..
                },
            )) => {
                if is_new_peer {
                    tracing::info!("📋 Inserted new peer into the routing table");
                    for address in addresses.iter() {
                        tracing::info!(%address, "📋 at");
                    }
                    self.metrics.increase_peer_count();
                } else {
                    tracing::info!("📋 Updated the addresses of a peer in the routing table");
                    for address in addresses.iter() {
                        tracing::info!(%address, "📋 at");
                    }
                }

                if let Some(peer) = old_peer {
                    tracing::info!(peer_id = %peer, "📋 Removed peer from the routing table");
                    self.metrics.decrease_peer_count();
                }
            }

            // Ping responses
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Ping(
                libp2p::ping::Event {
                    peer,
                    connection,
                    result,
                },
            )) => match result {
                Ok(rtt) => {
                    tracing::info!(%connection, "🔔 Dialing");
                    tracing::info!(
                        peer_id = %peer,
                        rtt = rtt.as_secs(),
                        "🔔 Received ping response from peer"
                    );
                }
                Err(_) => {
                    tracing::info!(%connection, "🔕 Dialing");
                    tracing::error!(peer_id = %peer, "🔕 Failed to get ping response from peer");
                }
            },

            // New attestation gossip
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Gossipsub(
                libp2p::gossipsub::Event::Message {
                    propagation_source,
                    message_id,
                    message,
                },
            )) => {
                // Update metrics
                self.metrics.increase_gossipsub_message_count();

                let decode = common::types::Attestation::decode(&mut message.data.as_ref());
                let Ok(attestation) = decode else {
                    tracing::error!(peer_id = %propagation_source, "⛔ Received invalid attestation");
                    self.metrics.increase_invalid_gossipsub_count();

                    self.swarm
                        .behaviour_mut()
                        .gossipsub
                        .report_message_validation_result(
                            &message_id,
                            &propagation_source,
                            libp2p::gossipsub::MessageAcceptance::Reject,
                        );

                    return Ok(());
                };

                let digest = attestation.digest();

                tracing::info!(
                    peer_id = %propagation_source,
                    %digest,
                    height = attestation.header_number(),
                    attestor_id = %attestation.attestor,
                    "📩 Received attestation"
                );

                match self.sender_validation.send(attestation).transpose() {
                    // CASE 1] ACCEPT
                    //
                    // Valid insertions or failures which depend on local state are still propagated
                    // to the rest of the network.
                    Ok(_) => {
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                libp2p::gossipsub::MessageAcceptance::Accept,
                            );
                    }
                    Err(err @ crate::worker::validation::pool::Error::NoSpaceLeft(..)) => {
                        err.log_error(digest);
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                libp2p::gossipsub::MessageAcceptance::Accept,
                            );
                    }
                    // CASE 2] IGNORE
                    //
                    // Failures which depend on finality lag are not considered as malicious but are
                    // not propagated to the rest of the network as they are out-of-date.
                    Err(err @ crate::worker::validation::pool::Error::InvalidHeight(..))
                    | Err(err @ crate::worker::validation::pool::Error::InvalidDigest(..)) => {
                        err.log_error(digest);
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                libp2p::gossipsub::MessageAcceptance::Ignore,
                            );
                    }
                    Err(err @ crate::worker::validation::pool::Error::Equivocation(..)) => {
                        err.log_error(digest);
                        self.metrics.increase_equivocation_count();
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                libp2p::gossipsub::MessageAcceptance::Ignore,
                            );
                    }
                    // CASE 3] REJECT
                    //
                    // Failures which depend solely on the sender are considered malicious. They are
                    // not propagated to the rest of the network.
                    Err(err @ crate::worker::validation::pool::Error::Unauthorized(..)) => {
                        err.log_error(digest);
                        self.swarm
                            .behaviour_mut()
                            .gossipsub
                            .report_message_validation_result(
                                &message_id,
                                &propagation_source,
                                libp2p::gossipsub::MessageAcceptance::Reject,
                            );
                    }
                }
            }

            // New gossipsub subscriptions (happens when a new peer joins)
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Gossipsub(
                libp2p::gossipsub::Event::Subscribed { .. },
            )) => {
                let topic_attestation = self.topic.hash();
                self.can_broadcast = self
                    .swarm
                    .behaviour()
                    .gossipsub
                    .mesh_peers(&topic_attestation)
                    .next()
                    .is_some();
            }

            // Remove gossipsub subscription (happens when an existing peer leaves)
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Gossipsub(
                libp2p::gossipsub::Event::Unsubscribed { .. },
            )) => {
                let topic_attestation = self.topic.hash();
                self.can_broadcast = self
                    .swarm
                    .behaviour()
                    .gossipsub
                    .mesh_peers(&topic_attestation)
                    .next()
                    .is_some();
            }

            // Discovered new local address
            libp2p::swarm::SwarmEvent::NewListenAddr {
                listener_id,
                address,
            } => {
                if let Ok(address) = address.with_p2p(*self.swarm.local_peer_id()) {
                    tracing::info!(%listener_id, %address, "🔍 New listening address")
                }
            }

            // Connected to a new remote peer
            libp2p::swarm::SwarmEvent::ConnectionEstablished {
                peer_id,
                connection_id,
                ..
            } => {
                tracing::info!(connection = %connection_id, "🔗 Dialing");
                tracing::info!(%peer_id, "🔗 Connection established");
            }

            // Disconnected from an existing remote peer
            libp2p::swarm::SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                ..
            } => {
                tracing::info!(connection = %connection_id, "⛓️‍💥 Dialing");
                tracing::info!(%peer_id, "⛓️‍💥 Connection closed");

                let topic_attestation = self.topic.hash();
                self.can_broadcast = self
                    .swarm
                    .behaviour()
                    .gossipsub
                    .mesh_peers(&topic_attestation)
                    .next()
                    .is_some();
            }

            // Failed to initialize a new connection with a remote peer
            libp2p::swarm::SwarmEvent::OutgoingConnectionError {
                peer_id,
                error,
                connection_id,
            } => {
                tracing::error!(connection = %connection_id, "⛔ Dialing");
                tracing::error!(?peer_id, %error, "⛔  Outgoing connection error");

                // Update metrics
                self.metrics.increase_connection_failure_count();

                // WARNING: ERROR HANDLING
                //
                // We only remove the peer from the routing table in the case of an
                // unrecoverable error.
                match error {
                    libp2p::swarm::DialError::Aborted => {
                        tracing::error!("⛔  Connection aborted");
                    }
                    libp2p::swarm::DialError::LocalPeerId { .. } => {
                        tracing::error!("⛔  Tried to dial self");
                    }
                    libp2p::swarm::DialError::DialPeerConditionFalse(peer_condition) => {
                        tracing::error!(?peer_condition, "⛔  Invalid peer state");
                    }
                    libp2p::swarm::DialError::NoAddresses => {
                        tracing::error!("⛔  Tried to dial empty address");
                    }
                    libp2p::swarm::DialError::Transport(items) => {
                        // TODO: perhaps we should remove peer addresses in this case
                        for (address, error) in items.iter() {
                            tracing::error!(%address, %error, "⛔  Failed transport negotiation");
                        }
                    }

                    // NOTE: WrongPeerId is an unrecoverable error.
                    //
                    // During the initial connection handshake, libp2p peers exchange a proof of
                    // ownership of the PeerId they are broadcasting. This is possible as the PeerId
                    // is a commitment to an attestor's private key. For an attestor to fail PeerID
                    // verification means it is impersonating another attestor, and should be
                    // considered malicious.
                    libp2p::swarm::DialError::WrongPeerId { obtained, address } => {
                        tracing::error!(%obtained, expected =  %address, "⛔  Peer ID missmatch");

                        if let Some(peer_id) = peer_id {
                            self.swarm.behaviour_mut().kad.remove_peer(&peer_id);
                        }
                    }

                    // NOTE: Denied is an unrecoverable error
                    //
                    // Connection was established, but the peer refused to connect to us.
                    libp2p::swarm::DialError::Denied { cause } => {
                        tracing::error!(%cause, "⛔  Connection denied");

                        if let Some(peer_id) = peer_id {
                            self.swarm.behaviour_mut().kad.remove_peer(&peer_id);
                        }
                    }
                }
            }

            _ => (),
        };

        Ok(())
    }

    async fn handle_event_shutdown(&mut self) -> common::types::Result<()> {
        Ok(())
    }
}
