//! A [`Worker`] thread responsible for the dissemination and reception of new attestations.
//!
//! # Gossiping
//!
//! Attestations are gossiped between attestor nodes via a custom [gossipsub] network. Attestations
//! are scoped per source chain, so that network traffic for a given source chain does not impact
//! the performance of attestation production for another chain. Currently only Ethereum is
//! supported.
//!
//! # Reception
//!
//! The p2p worker is also responsible for synchronizing new attestations it receives via gossipsub.
//! On noticing a new attestation, the p2p worker will try to deserialize it and, if successful,
//! send it to the [attestation pool] for further [`Quorum`] checks. Attestations which fail to be
//! inserted into the attestation pool are considered invalid and potentially malicious and are not
//! forwarded to the rest of the network.
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
//! [production worker]: crate::worker::production
//! [attestation pool]: attestation_pool
//! [`Quorum`]: attestation_pool::Quorum

pub(crate) mod auth;

mod behavior;
mod error;
mod protocols;

pub use auth::PeerAuth;
pub use error::*;
use user::prelude::*;

/// How long a peer has to complete the authorization handshake after connecting before it is
/// disconnected and blocklisted.
const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// How often unverified connections are swept for [`AUTH_TIMEOUT`] expiry.
const AUTH_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(builder::Builder)]
pub struct Config {
    boot_nodes: Vec<libp2p::Multiaddr>,
    public_addr: Option<String>,
    port: u16,
    #[default(false)]
    no_mdns: bool,
    /// Disables peer authorization enforcement. Intended only for the rolling-upgrade window,
    /// before every node on the network speaks the [`AUTH`] protocol. See [`AUTH`].
    ///
    /// [`AUTH`]: protocols::AUTH
    #[default(false)]
    no_peer_auth: bool,
    #[specify_later]
    bls: std::sync::Arc<crate::bls::BlsStore>,
    /// Our own proof-of-possession, presented to peers on connection.
    #[specify_later]
    self_auth: auth::PeerAuth,
    #[specify_later]
    keypair: libp2p::identity::Keypair,
    #[specify_later]
    receiver_p2p: tokio::sync::broadcast::Receiver<common::types::Attestation>,
    #[specify_later]
    sender_validation: attestation_pool::AttestationPoolSender,
    #[specify_later]
    chain_key: attestor_primitives::ChainKey,
    #[specify_later]
    metrics: metrics::Metrics,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub(crate) struct WorkerP2P {
    // P2P DATA
    swarm: libp2p::Swarm<behavior::P2PBehavior>,
    can_broadcast: bool,
    topic: libp2p::gossipsub::IdentTopic,
    listen_addr: libp2p::Multiaddr,

    chain_key: attestor_primitives::ChainKey,
    // CC3 CONNECTION
    bls: std::sync::Arc<crate::bls::BlsStore>,

    // PEER AUTHORIZATION
    /// Whether to enforce the BLS proof-of-possession handshake on incoming peers.
    auth_required: bool,
    /// Our own proof-of-possession, sent to peers on connection.
    self_auth: auth::PeerAuth,
    /// Peers which have connected but not yet completed the authorization handshake, with the
    /// instant they connected. Swept on [`AUTH_TIMEOUT`].
    pending_auth: std::collections::HashMap<libp2p::PeerId, std::time::Instant>,
    /// Peers that have completed the handshake and remain connected. Tracked so that additional
    /// connections to (or reconnections from) an already-trusted peer don't re-run the timed
    /// handshake and risk blocklisting a legitimate peer.
    verified: std::collections::HashSet<libp2p::PeerId>,

    // METRICS
    metrics: metrics::Metrics,

    // MESSAGE CHANNELS
    receiver_p2p: tokio::sync::broadcast::Receiver<common::types::Attestation>,
    sender_validation: attestation_pool::AttestationPoolSender,
}

impl WorkerP2P {
    pub(crate) async fn new(config: Config) -> anyhow::Result<Self> {
        let enable_mdns = !config.no_mdns;
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
            .with_behaviour(|key| behavior::P2PBehavior::new(key, enable_mdns))?
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

            chain_key: config.chain_key,
            bls: config.bls,

            auth_required: !config.no_peer_auth,
            self_auth: config.self_auth,
            pending_auth: std::collections::HashMap::new(),
            verified: std::collections::HashSet::new(),

            metrics: config.metrics,

            receiver_p2p: config.receiver_p2p,
            sender_validation: config.sender_validation,
        })
    }
}

// ---------------------------------------- [ Main loop ] -------------------------------------- //

impl super::Worker for WorkerP2P {
    type Error = Error;

    #[tracing::instrument(name = "p2p_engine", skip_all)]
    async fn task(
        mut self,
        mut shutdown: std::pin::Pin<Box<impl std::future::Future<Output = ()>>>,
    ) -> crate::worker::Exit<Error> {
        use futures::StreamExt as _;

        // Tell the swarm to listen on all interfaces on the configured port.
        // Default port is 9000, which is useful for Kubernetes LoadBalancer services.
        self.swarm
            .listen_on(self.listen_addr.clone())
            .map_interrupt(Error::Transport)?;

        if self.auth_required {
            tracing::info!("🔐 Peer authorization enforcement enabled");
        } else {
            tracing::warn!("🔓 Peer authorization enforcement DISABLED");
        }

        let mut auth_sweep = tokio::time::interval(AUTH_SWEEP_INTERVAL);

        loop {
            tokio::select! {
                biased;

                _ = &mut shutdown => {
                    break Err(Interrupt::Stop);
                }
                _ = auth_sweep.tick(), if self.auth_required => {
                    self.sweep_pending_auth();
                }
                attestation = self.receiver_p2p.recv(), if self.can_broadcast => {
                    self.handle_event_attestation(attestation).await.map_err(Interrupt::Cont)?;
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
                ?digest,
                height,
                attestor_id = %attestation.attestor,
                "✉️ Gossiping attestation"
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
    ) -> Result<(), Interrupt<Error>> {
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
                tracing::info!(%peer_id, connection = %connection_id, "🛰️ Discovered new remote peer");
                for address in listen_addrs {
                    tracing::info!(%peer_id, %address, "🛰️ Adding remote peer address");
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
                    tracing::info!(%peer_id, %address, "🛰️ Discovered new local peer");

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
                    peer,
                    is_new_peer,
                    addresses,
                    old_peer,
                    ..
                },
            )) => {
                if is_new_peer {
                    tracing::info!(
                        peer_id = %peer,
                        addresses = addresses.len(),
                        "📋 Inserted new peer into the routing table"
                    );
                    self.metrics.increase_peer_count();
                } else {
                    tracing::info!(
                        peer_id = %peer,
                        addresses = addresses.len(),
                        "📋 Updated addresses of a peer in the routing table"
                    );
                }

                if let Some(evicted_peer) = old_peer {
                    tracing::info!(peer_id = %evicted_peer, "📋 Removed peer from the routing table");
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
                    tracing::info!(
                        peer_id = %peer,
                        connection = %connection,
                        rtt_ms = rtt.as_millis(),
                        "🔔 Received ping response from peer"
                    );
                }
                Err(err) => {
                    tracing::error!(
                        peer_id = %peer,
                        connection = %connection,
                        %err,
                        "🔕 Failed to get ping response from peer"
                    );
                }
            },

            // Peer authorization handshake
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Auth(
                libp2p::request_response::Event::Message { peer, message, .. },
            )) => {
                self.handle_event_auth(peer, message).await;
            }

            // The handshake could not be completed (unsupported protocol, timeout, dropped
            // connection, ...). A peer we cannot authorize is not allowed on the network.
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Auth(
                libp2p::request_response::Event::OutboundFailure { peer, error, .. },
            )) => {
                tracing::warn!(peer_id = %peer, ?error, "⛔ Peer authorization request failed");
                self.reject_peer(peer);
            }
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Auth(
                libp2p::request_response::Event::InboundFailure { peer, error, .. },
            )) => {
                tracing::warn!(peer_id = %peer, ?error, "⛔ Peer authorization response failed");
                self.reject_peer(peer);
            }
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Auth(
                libp2p::request_response::Event::ResponseSent { .. },
            )) => {}

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
                let height = attestation.header_number();
                let attestor_id = attestation.attestor_id();

                tracing::info!(
                    peer_id = %propagation_source,
                    ?digest,
                    height,
                    %attestor_id,
                    "📩 Received attestation"
                );

                match self.validate_attestation(&attestation).await {
                    Err(Interrupt::Cont(err)) => {
                        tracing::error!(?err, "⛔ Invalid attestation");

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
                    }
                    Err(Interrupt::Stop) => return Err(Interrupt::Stop),
                    _ => {
                        tracing::debug!(
                            ?digest,
                            height,
                            %attestor_id,
                            "Valid attestation BLS signature"
                        );
                    }
                };

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
                    Err(err @ attestation_pool::Error::NoSpaceLeft(..)) => {
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
                    Err(err @ attestation_pool::Error::InvalidHeight(..))
                    | Err(err @ attestation_pool::Error::InvalidDigest(..)) => {
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
                    Err(err @ attestation_pool::Error::Equivocation(..)) => {
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
                    Err(err @ attestation_pool::Error::Unauthorized(..)) => {
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
                tracing::info!(%peer_id, connection = %connection_id, "🔗 Connection established");

                if self.auth_required && !self.verified.contains(&peer_id) {
                    // Start the authorization clock for this peer.
                    let first_connection = self
                        .pending_auth
                        .insert(peer_id, std::time::Instant::now())
                        .is_none();

                    // Both sides proactively challenge each other (not just the dialer). This is
                    // what bounds the abuse window: a peer that does not speak `/gluwa/auth` fails
                    // our request immediately (`OutboundFailure::UnsupportedProtocols`) and is
                    // rejected in milliseconds, rather than lingering — and flooding gossip — until
                    // the [`AUTH_TIMEOUT`] sweep. We only send once per peer to avoid redundant
                    // requests across multiple connections.
                    if first_connection {
                        self.swarm
                            .behaviour_mut()
                            .auth
                            .send_request(&peer_id, self.self_auth.clone());
                    }
                }
            }

            // Disconnected from an existing remote peer
            libp2p::swarm::SwarmEvent::ConnectionClosed {
                peer_id,
                connection_id,
                ..
            } => {
                tracing::info!(%peer_id, connection = %connection_id, "⛓️‍💥 Connection closed");

                self.pending_auth.remove(&peer_id);
                // Once the peer is fully disconnected it must re-authorize on reconnect.
                if !self.swarm.is_connected(&peer_id) {
                    self.verified.remove(&peer_id);
                }

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
                tracing::error!(?peer_id, connection = %connection_id, %error, "⛔ Outgoing connection error");

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
                        tracing::error!(%obtained, expected = %address, "⛔  Peer ID mismatch");

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

    // ------------------------------------* Authorization *--------------------------------------

    /// Handles an inbound peer-auth message. A [`Request`] is the peer proving itself to us; on
    /// success we reply with our own proof so the peer can verify us in turn. A [`Response`] is the
    /// peer proving itself in reply to our request. Either way, a peer that cannot prove membership
    /// of the on-chain attestor set is disconnected and blocklisted.
    ///
    /// [`Request`]: libp2p::request_response::Message::Request
    /// [`Response`]: libp2p::request_response::Message::Response
    async fn handle_event_auth(
        &mut self,
        peer: libp2p::PeerId,
        message: libp2p::request_response::Message<auth::PeerAuth, auth::PeerAuth>,
    ) {
        match message {
            libp2p::request_response::Message::Request {
                request, channel, ..
            } => {
                if self.verify_peer(&peer, &request).await {
                    // Reply with our own proof so the dialer can authorize us, then trust the peer.
                    if self
                        .swarm
                        .behaviour_mut()
                        .auth
                        .send_response(channel, self.self_auth.clone())
                        .is_err()
                    {
                        tracing::warn!(peer_id = %peer, "⚠️ Failed to send authorization response");
                    }
                    self.mark_verified(peer);
                } else {
                    // Dropping `channel` signals an inbound failure to the peer; we also reject it.
                    self.reject_peer(peer);
                }
            }
            libp2p::request_response::Message::Response { response, .. } => {
                if self.verify_peer(&peer, &response).await {
                    self.mark_verified(peer);
                } else {
                    self.reject_peer(peer);
                }
            }
        }
    }

    /// Verifies a peer's proof-of-possession against the on-chain attestor set and the connection's
    /// authenticated [`PeerId`]. Returns `false` if the claimed attestor is not authorized or the
    /// BLS signature does not verify.
    ///
    /// [`PeerId`]: libp2p::PeerId
    async fn verify_peer(&self, peer: &libp2p::PeerId, proof: &auth::PeerAuth) -> bool {
        let Some(pubkey) = self
            .bls
            .pubkey(cc_client::AccountId32(proof.attestor))
            .await
        else {
            tracing::warn!(peer_id = %peer, "⛔ Peer claims an unauthorized attestor identity");
            return false;
        };

        if proof.verify(&pubkey, self.chain_key, peer) {
            true
        } else {
            tracing::warn!(peer_id = %peer, "⛔ Peer presented an invalid authorization proof");
            false
        }
    }

    /// Marks a peer as authorized: clears its pending timer and adds it as an explicit gossipsub
    /// peer to ensure reliable attestation propagation.
    fn mark_verified(&mut self, peer: libp2p::PeerId) {
        self.pending_auth.remove(&peer);
        if self.verified.insert(peer) {
            tracing::info!(peer_id = %peer, "🔓 Peer authorized");
        }
        self.swarm
            .behaviour_mut()
            .gossipsub
            .add_explicit_peer(&peer);
    }

    /// Disconnects and blocklists a peer that failed authorization, removing it from the routing
    /// table so it cannot be rediscovered. Blocklisted peers are denied at the swarm layer on any
    /// future connection attempt.
    fn reject_peer(&mut self, peer: libp2p::PeerId) {
        tracing::warn!(peer_id = %peer, "⛔ Blocklisting unauthorized peer");

        self.pending_auth.remove(&peer);
        self.verified.remove(&peer);
        self.swarm.behaviour_mut().blocked.block_peer(peer);
        self.swarm.behaviour_mut().kad.remove_peer(&peer);
        self.swarm
            .behaviour_mut()
            .gossipsub
            .remove_explicit_peer(&peer);
        let _ = self.swarm.disconnect_peer_id(peer);

        self.metrics.increase_connection_failure_count();
    }

    /// Disconnects and blocklists any peers that have not completed the authorization handshake
    /// within [`AUTH_TIMEOUT`]. This is the defense against peers that connect and stay silent to
    /// occupy connection slots or force per-message work.
    fn sweep_pending_auth(&mut self) {
        let now = std::time::Instant::now();
        let expired: Vec<libp2p::PeerId> = self
            .pending_auth
            .iter()
            .filter(|(_, connected_at)| now.duration_since(**connected_at) > AUTH_TIMEOUT)
            .map(|(peer, _)| *peer)
            .collect();

        for peer in expired {
            tracing::warn!(peer_id = %peer, "⛔ Peer failed to authorize within timeout");
            self.reject_peer(peer);
        }
    }

    /// Verifies attestor eligibility and attestation bls signature before submitting to the
    /// [attestation pool].
    ///
    /// [attestation pool]: attestation_pool
    async fn validate_attestation(
        &mut self,
        attestation: &common::types::Attestation,
    ) -> Result<(), Interrupt<Error>> {
        let attestor_id = attestation.attestor_id();
        let digest = attestation.digest();
        let chain_key = attestation.chain_key();

        // WARNING: while we use the chain_key as the topic id for gossip propagation, this
        // does not enforce that attestations received correspond to the correct chain key!
        // A malicious or dysfunctional attestor is still able to send attestation with a
        // chain key than the gossip topic, so this needs to be checked before pool
        // insertion.
        if chain_key != self.chain_key {
            return Err(Interrupt::Cont(Error::InvalidAttestation(
                InvalidCause::Unsupported(chain_key),
            )));
        }

        let Some(pubkey) = self.bls.pubkey(attestor_id.account_id()).await else {
            return Err(Interrupt::Cont(Error::InvalidAttestation(
                InvalidCause::Unregistered(attestor_id),
            )));
        };

        let msg = attestation.attestation_data.serialize();
        if pubkey.verify(attestation.signature_bls.0, &msg) {
            Ok(())
        } else {
            Err(Interrupt::Cont(Error::InvalidAttestation(
                InvalidCause::InvalidBls(digest),
            )))
        }
    }
}

// ------------------------------------- [ Auth network tests ] -------------------------------- //

/// End-to-end tests for the peer authorization handshake. Two real [`P2PBehavior`] swarms talk over
/// loopback TCP, exercising the same `request_response` + `allow_block_list` behaviours used in
/// production. The handshake glue here mirrors [`WorkerP2P::handle_event_p2p`] but verifies against
/// an in-memory authorized set instead of the on-chain [`BlsStore`], so no chain is required.
///
/// Run as an eyeball test with:
/// `cargo test -p attestor auth_net -- --nocapture`
///
/// [`P2PBehavior`]: behavior::P2PBehavior
/// [`BlsStore`]: crate::bls::BlsStore
#[cfg(test)]
mod auth_net_tests {
    use super::*;
    use futures::StreamExt as _;
    use std::collections::HashMap;

    const CHAIN_KEY: attestor_primitives::ChainKey = 2;

    /// A test node: its swarm, identity, own proof, and the set of attestors it considers
    /// authorized (a stand-in for the on-chain BLS set).
    struct Node {
        name: &'static str,
        swarm: libp2p::Swarm<behavior::P2PBehavior>,
        peer_id: libp2p::PeerId,
        self_auth: auth::PeerAuth,
        authorized: HashMap<[u8; 32], bls_signatures::PublicKey>,
    }

    /// What a node decided about a peer during the handshake.
    #[derive(Debug, PartialEq, Eq)]
    enum Decision {
        Authorized(libp2p::PeerId),
        Blocked(libp2p::PeerId),
    }

    fn bls_key(seed: &[u8]) -> bls_signatures::PrivateKey {
        let mut ikm = [0u8; 32];
        for (slot, byte) in ikm.iter_mut().zip(seed.iter().cycle()) {
            *slot = *byte;
        }
        bls_signatures::PrivateKey::new(ikm)
    }

    fn build_node(
        name: &'static str,
        attestor: [u8; 32],
        bls: &bls_signatures::PrivateKey,
    ) -> Node {
        let keypair = libp2p::identity::Keypair::generate_ed25519();
        let peer_id = keypair.public().to_peer_id();

        let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
            .with_tokio()
            .with_tcp(
                libp2p::tcp::Config::default(),
                libp2p::noise::Config::new,
                libp2p::yamux::Config::default,
            )
            .expect("tcp transport")
            .with_behaviour(|key| behavior::P2PBehavior::new(key, false))
            .expect("behaviour")
            .build();

        let self_auth = auth::PeerAuth::new(bls, attestor, CHAIN_KEY, &peer_id);

        Node {
            name,
            swarm,
            peer_id,
            self_auth,
            authorized: HashMap::new(),
        }
    }

    /// Verifies a peer's proof against this node's authorized set and the connection's
    /// authenticated `peer_id` — the test mirror of [`WorkerP2P::verify_peer`].
    fn verify(node: &Node, peer: &libp2p::PeerId, proof: &auth::PeerAuth) -> bool {
        match node.authorized.get(&proof.attestor) {
            Some(pubkey) => proof.verify(pubkey, CHAIN_KEY, peer),
            None => false,
        }
    }

    /// Applies one swarm event, mirroring the auth-relevant arms of
    /// [`WorkerP2P::handle_event_p2p`]. Returns a [`Decision`] when this node authorizes or blocks
    /// the peer.
    fn on_event(
        node: &mut Node,
        event: libp2p::swarm::SwarmEvent<behavior::P2PBehaviorEvent>,
    ) -> Option<Decision> {
        match event {
            libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                // Both sides proactively challenge, mirroring the production handler.
                node.swarm
                    .behaviour_mut()
                    .auth
                    .send_request(&peer_id, node.self_auth.clone());
                None
            }
            libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Auth(
                libp2p::request_response::Event::Message { peer, message, .. },
            )) => match message {
                libp2p::request_response::Message::Request {
                    request, channel, ..
                } => {
                    if verify(node, &peer, &request) {
                        let _ = node
                            .swarm
                            .behaviour_mut()
                            .auth
                            .send_response(channel, node.self_auth.clone());
                        println!("✅ {} authorized peer {peer}", node.name);
                        Some(Decision::Authorized(peer))
                    } else {
                        node.swarm.behaviour_mut().blocked.block_peer(peer);
                        let _ = node.swarm.disconnect_peer_id(peer);
                        println!("⛔ {} BLOCKLISTED rogue peer {peer}", node.name);
                        Some(Decision::Blocked(peer))
                    }
                }
                libp2p::request_response::Message::Response { response, .. } => {
                    if verify(node, &peer, &response) {
                        println!("✅ {} authorized peer {peer}", node.name);
                        Some(Decision::Authorized(peer))
                    } else {
                        node.swarm.behaviour_mut().blocked.block_peer(peer);
                        let _ = node.swarm.disconnect_peer_id(peer);
                        println!("⛔ {} BLOCKLISTED rogue peer {peer}", node.name);
                        Some(Decision::Blocked(peer))
                    }
                }
            },
            _ => None,
        }
    }

    /// Has `dialer` connect to `listener` and drives both swarms until the listener reaches a
    /// decision about the dialer (or a timeout). Returns the listener's decision.
    async fn run_handshake(mut listener: Node, mut dialer: Node) -> Option<Decision> {
        listener
            .swarm
            .listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap())
            .expect("listen");

        // Wait for the listen address, then dial it.
        let addr = loop {
            if let libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } =
                listener.swarm.select_next_some().await
            {
                break address;
            }
        };
        println!("🔭 {} listening on {addr}", listener.name);
        println!("📞 {} dialing {}", dialer.name, listener.name);
        dialer.swarm.dial(addr).expect("dial");

        let timeout = tokio::time::sleep(std::time::Duration::from_secs(10));
        tokio::pin!(timeout);

        loop {
            tokio::select! {
                _ = &mut timeout => {
                    println!("⌛ handshake timed out");
                    break None;
                }
                event = listener.swarm.select_next_some() => {
                    if let Some(decision) = on_event(&mut listener, event) {
                        break Some(decision);
                    }
                }
                event = dialer.swarm.select_next_some() => {
                    let _ = on_event(&mut dialer, event);
                }
            }
        }
    }

    #[tokio::test]
    async fn authorized_peer_completes_handshake() {
        let listener_bls = bls_key(b"listener-attestor-seed");
        let dialer_bls = bls_key(b"dialer-attestor-seed");
        let listener_id = [1u8; 32];
        let dialer_id = [2u8; 32];

        let mut listener = build_node("listener", listener_id, &listener_bls);
        let mut dialer = build_node("dialer", dialer_id, &dialer_bls);

        // Both nodes are in each other's authorized set: a healthy attestor network.
        listener
            .authorized
            .insert(dialer_id, dialer_bls.public_key());
        dialer
            .authorized
            .insert(listener_id, listener_bls.public_key());

        let dialer_peer = dialer.peer_id;
        let decision = run_handshake(listener, dialer).await;

        assert_eq!(decision, Some(Decision::Authorized(dialer_peer)));
    }

    #[tokio::test]
    async fn rogue_peer_is_blocklisted() {
        let listener_bls = bls_key(b"listener-attestor-seed");
        let rogue_bls = bls_key(b"rogue-attestor-seed");
        let listener_id = [1u8; 32];
        let rogue_id = [99u8; 32];

        let listener = build_node("listener", listener_id, &listener_bls);
        let rogue = build_node("rogue", rogue_id, &rogue_bls);

        // The rogue's attestor identity is NOT in the listener's authorized set: it is not part of
        // the on-chain attestor set. It presents a perfectly-formed proof, but for an unauthorized
        // identity.
        let rogue_peer = rogue.peer_id;
        let decision = run_handshake(listener, rogue).await;

        assert_eq!(decision, Some(Decision::Blocked(rogue_peer)));
    }
}
