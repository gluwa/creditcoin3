mod behavior;
mod error;
mod metrics;
mod protocols;

pub use error::*;
pub use metrics::*;

// -------------------------------------- [ Configuration ] ------------------------------------ //

#[derive(builder::Builder)]
pub struct Config {
    boot_nodes: Vec<libp2p::Multiaddr>,
    public_addr: Option<String>,
    port: u16,
    #[default(false)]
    no_mdns: bool,
    #[specify_later]
    keypair: libp2p::identity::Keypair,
    #[specify_later]
    chain_key: attestor_primitives::ChainKey,
    #[specify_later]
    metrics: Box<dyn Metrics>,
}

// ----------------------------------------- [ Worker ] ---------------------------------------- //

pub struct P2PNetwork {
    // P2P DATA
    swarm: libp2p::Swarm<behavior::P2PBehavior>,
    can_broadcast: bool,
    topic: libp2p::gossipsub::IdentTopic,

    // Sink
    waker: Option<std::task::Waker>,

    // METRICS
    metrics: Box<dyn Metrics>,
}

impl P2PNetwork {
    pub async fn new(config: Config) -> anyhow::Result<Self> {
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

        // Tell the swarm to listen on all interfaces on the configured port.
        // Default port is 9000, which is useful for Kubernetes LoadBalancer services.
        swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", config.port).parse()?)?;

        Ok(Self {
            swarm,
            can_broadcast: false,
            topic,

            waker: None,

            metrics: config.metrics,
        })
    }

    fn update_broadcast_status(&mut self) {
        let topic_attestation = self.topic.hash();
        let can_broadcast = self
            .swarm
            .behaviour()
            .gossipsub
            .mesh_peers(&topic_attestation)
            .next()
            .is_some();

        self.can_broadcast = can_broadcast;

        if let Some(waker) = self.waker.take_if(|_| can_broadcast) {
            waker.wake()
        }
    }
}

impl futures::Sink<common::types::Attestation> for P2PNetwork {
    type Error = Error;

    fn poll_ready(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        if self.can_broadcast {
            std::task::Poll::Ready(Ok(()))
        } else {
            match &self.waker {
                Some(w) if w.will_wake(cx.waker()) => {}
                _ => self.waker = Some(cx.waker().clone()),
            }
            std::task::Poll::Pending
        }
    }

    fn start_send(
        mut self: std::pin::Pin<&mut Self>,
        attestation: common::types::Attestation,
    ) -> Result<(), Self::Error> {
        use parity_scale_codec::Encode as _;

        let digest = attestation.digest();
        let height = attestation.header_number();

        tracing::info!(
            ?digest,
            height,
            attestor_id = %attestation.attestor,
            "✉️ Gossiping attestation"
        );

        let topic = self.topic.hash();

        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(topic, attestation.encode())
            .map_err(|err| Error::PublishError(height, digest, err))?;

        Ok(())
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }
}

impl futures::Stream for P2PNetwork {
    type Item = common::types::Attestation;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;
        use parity_scale_codec::Decode as _;

        loop {
            let Some(event) = std::task::ready!(self.swarm.poll_next_unpin(cx)) else {
                return std::task::Poll::Ready(None);
            };

            match event {
                // Discovering new remote peers with kad + identify
                libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Identify(
                    libp2p::identify::Event::Received {
                        connection_id,
                        peer_id,
                        info: libp2p::identify::Info { listen_addrs, .. },
                    },
                )) => {
                    tracing::info!(
                        %peer_id,
                        connection = %connection_id,
                        "🛰️ Discovered new remote peer"
                    );

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
                        tracing::info!(
                            peer_id = %evicted_peer,
                            "📋 Removed peer from the routing table"
                        );
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

                    match decode {
                        Ok(attestation) => {
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

                            return std::task::Poll::Ready(Some(attestation));
                        }
                        Err(err) => {
                            tracing::error!(
                                peer_id = %propagation_source,
                                %err,
                                "⛔ Received invalid attestation"
                            );

                            self.metrics.increase_invalid_gossipsub_count();
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
                    self.update_broadcast_status();
                }

                // Remove gossipsub subscription (happens when an existing peer leaves)
                libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Gossipsub(
                    libp2p::gossipsub::Event::Unsubscribed { .. },
                )) => {
                    self.update_broadcast_status();
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
                    tracing::info!(
                        %peer_id,
                        connection = %connection_id,
                        "🔗 Connection established"
                    );
                }

                // Disconnected from an existing remote peer
                libp2p::swarm::SwarmEvent::ConnectionClosed {
                    peer_id,
                    connection_id,
                    ..
                } => {
                    tracing::info!(%peer_id, connection = %connection_id, "⛓️‍💥 Connection closed");
                    self.update_broadcast_status();
                }

                // Failed to initialize a new connection with a remote peer
                libp2p::swarm::SwarmEvent::OutgoingConnectionError {
                    peer_id,
                    error,
                    connection_id,
                } => {
                    tracing::error!(
                        ?peer_id,
                        connection = %connection_id,
                        %error, "⛔ Outgoing connection error"
                    );

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
                                tracing::error!(
                                    %address,
                                    %error,
                                    "⛔  Failed transport negotiation"
                                );
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
                            tracing::error!(
                                %obtained,
                                expected = %address,
                                "⛔  Peer ID mismatch"
                            );

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
        }
    }
}
