mod behavior;
mod error;
mod protocols;

pub use error::Error;

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
    metrics: metrics::Metrics,
}

pub struct StreamP2P<Message>
where
    Message: parity_scale_codec::Encode + parity_scale_codec::Decode + Unpin,
{
    // P2P DATA
    swarm: libp2p::Swarm<behavior::P2PBehavior>,
    can_broadcast: bool,
    topic: libp2p::gossipsub::IdentTopic,

    // METRICS
    metrics: metrics::Metrics,

    // MESSAGES
    _phatom: std::marker::PhantomData<Message>,
}

impl<Message> StreamP2P<Message>
where
    Message: parity_scale_codec::Encode + parity_scale_codec::Decode + Unpin,
{
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

        swarm.listen_on(format!("/ip4/0.0.0.0/tcp/{}", config.port).parse()?)?;

        Ok(Self {
            swarm,
            can_broadcast: false,
            topic,

            metrics: config.metrics,
            _phatom: std::marker::PhantomData,
        })
    }

    pub fn send(&mut self, data: Message) -> Result<(), Error> {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic.hash(), data.encode())
            .map_err(|err| Error::PublishError(err))?;

        Ok(())
    }

    pub fn accept(
        &mut self,
        MessageValidate {
            message,
            message_id,
            propagation_source,
        }: MessageValidate<Message>,
    ) -> Message {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .report_message_validation_result(
                &message_id,
                &propagation_source,
                libp2p::gossipsub::MessageAcceptance::Accept,
            );

        message
    }

    pub fn ignore(
        &mut self,
        MessageValidate {
            message,
            message_id,
            propagation_source,
        }: MessageValidate<Message>,
    ) -> Message {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .report_message_validation_result(
                &message_id,
                &propagation_source,
                libp2p::gossipsub::MessageAcceptance::Ignore,
            );

        message
    }

    pub fn reject(
        &mut self,
        MessageValidate {
            message,
            message_id,
            propagation_source,
        }: MessageValidate<Message>,
    ) -> Message {
        self.swarm
            .behaviour_mut()
            .gossipsub
            .report_message_validation_result(
                &message_id,
                &propagation_source,
                libp2p::gossipsub::MessageAcceptance::Reject,
            );

        message
    }
}

impl<Message> futures::Stream for StreamP2P<Message>
where
    Message: parity_scale_codec::Encode + parity_scale_codec::Decode + Unpin,
{
    type Item = MessageValidate<Message>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt as _;

        loop {
            match std::task::ready!(self.swarm.poll_next_unpin(cx)) {
                // Discovering new remote peers with kad + identify
                Some(libp2p::swarm::SwarmEvent::Behaviour(
                    behavior::P2PBehaviorEvent::Identify(libp2p::identify::Event::Received {
                        connection_id,
                        peer_id,
                        info: libp2p::identify::Info { listen_addrs, .. },
                    }),
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
                Some(libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Mdns(
                    libp2p::mdns::Event::Discovered(peers),
                ))) => {
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
                Some(libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Kad(
                    libp2p::kad::Event::RoutingUpdated {
                        peer,
                        is_new_peer,
                        addresses,
                        old_peer,
                        ..
                    },
                ))) => {
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
                Some(libp2p::swarm::SwarmEvent::Behaviour(behavior::P2PBehaviorEvent::Ping(
                    libp2p::ping::Event {
                        peer,
                        connection,
                        result,
                    },
                ))) => match result {
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
                Some(libp2p::swarm::SwarmEvent::Behaviour(
                    behavior::P2PBehaviorEvent::Gossipsub(libp2p::gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message,
                    }),
                )) => {
                    // Update metrics
                    self.metrics.increase_gossipsub_message_count();

                    let Ok(message) = Message::decode(&mut message.data.as_ref()) else {
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

                        continue;
                    };

                    return std::task::Poll::Ready(Some(MessageValidate {
                        message,
                        message_id,
                        propagation_source,
                    }));
                }

                // New gossipsub subscriptions (happens when a new peer joins)
                Some(libp2p::swarm::SwarmEvent::Behaviour(
                    behavior::P2PBehaviorEvent::Gossipsub(libp2p::gossipsub::Event::Subscribed {
                        ..
                    }),
                )) => {
                    let topic_attestation = self.topic.hash();
                    let can_broadcast = self
                        .swarm
                        .behaviour()
                        .gossipsub
                        .mesh_peers(&topic_attestation)
                        .next()
                        .is_some();
                    self.can_broadcast = can_broadcast;
                }

                // Remove gossipsub subscription (happens when an existing peer leaves)
                Some(libp2p::swarm::SwarmEvent::Behaviour(
                    behavior::P2PBehaviorEvent::Gossipsub(libp2p::gossipsub::Event::Unsubscribed {
                        ..
                    }),
                )) => {
                    let topic_attestation = self.topic.hash();
                    let can_broadcast = self
                        .swarm
                        .behaviour()
                        .gossipsub
                        .mesh_peers(&topic_attestation)
                        .next()
                        .is_some();
                    self.can_broadcast = can_broadcast;
                }

                // Discovered new local address
                Some(libp2p::swarm::SwarmEvent::NewListenAddr {
                    listener_id,
                    address,
                }) => {
                    if let Ok(address) = address.with_p2p(*self.swarm.local_peer_id()) {
                        tracing::info!(%listener_id, %address, "🔍 New listening address")
                    }
                }

                // Connected to a new remote peer
                Some(libp2p::swarm::SwarmEvent::ConnectionEstablished {
                    peer_id,
                    connection_id,
                    ..
                }) => {
                    tracing::info!(%peer_id, connection = %connection_id, "🔗 Connection established");
                }

                // Disconnected from an existing remote peer
                Some(libp2p::swarm::SwarmEvent::ConnectionClosed {
                    peer_id,
                    connection_id,
                    ..
                }) => {
                    tracing::info!(%peer_id, connection = %connection_id, "⛓️‍💥 Connection closed");

                    let topic_attestation = self.topic.hash();
                    let can_broadcast = self
                        .swarm
                        .behaviour()
                        .gossipsub
                        .mesh_peers(&topic_attestation)
                        .next()
                        .is_some();
                    self.can_broadcast = can_broadcast;
                }

                // Failed to initialize a new connection with a remote peer
                Some(libp2p::swarm::SwarmEvent::OutgoingConnectionError {
                    peer_id,
                    error,
                    connection_id,
                }) => {
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
            }
        }
    }
}

pub struct MessageValidate<Message>
where
    Message: parity_scale_codec::Encode + parity_scale_codec::Decode,
{
    message: Message,
    message_id: libp2p::gossipsub::MessageId,
    propagation_source: libp2p::PeerId,
}

impl<Message> AsRef<Message> for MessageValidate<Message>
where
    Message: parity_scale_codec::Encode + parity_scale_codec::Decode,
{
    fn as_ref(&self) -> &Message {
        &self.message
    }
}
