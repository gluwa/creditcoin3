#[derive(libp2p::swarm::NetworkBehaviour)]
pub(crate) struct P2PBehavior {
    /// [`Ping`] keeps connections live and surfaces failed pings. Modern libp2p no longer closes
    /// a connection on repeated ping failures, so the gossip task tracks failures per connection
    /// and reaps wedged connections itself (see `MAX_PING_FAILURES`).
    ///
    /// [`Ping`]: libp2p::ping
    pub ping: libp2p::ping::Behaviour,

    /// [`Limits`] are used to enforce a max number of connections per peer.
    ///
    /// [`Limits`]: libp2p::connection_limits
    pub limits: libp2p::connection_limits::Behaviour,

    /// [`Identify`] is used for identification between peers. This is required as other protocols
    /// in this behavior do not implement identification of their own.
    ///
    /// [`Identify`]: libp2p::identify
    pub identify: libp2p::identify::Behaviour,

    /// [`mDNS`] is used for _local_ node discovery. This is handy for testing or setting up clusters
    /// under the same local network but doesn't solve for global peer discovery. Note that this
    /// tends to not works on K8s networks, in which case manual boot node registration via
    /// [`kademlia`] will be required to bootstrap the network.
    ///
    /// [`mDNS`]: libp2p::mdns
    /// [`kademlia`]: libp2p::kad
    pub mdns: libp2p::swarm::behaviour::toggle::Toggle<libp2p::mdns::tokio::Behaviour>,

    /// [`Kademlia`] is used for _global_ peer discovery. We use kademlia instead of [`rendezvous`]
    /// for its resilience to centralized points of failure as well as its in-build peer discovery.
    ///
    /// [`Kademlia`]: libp2p::kad
    /// [`rendezvous`]: https://github.com/libp2p/specs/blob/master/rendezvous/README.md
    pub kad: libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>,

    /// [`gossipsub`] is used for message passing between attestor nodes across the same p2p
    /// network, allowing for the exchange of new attestations and network updates. Messages
    /// disseminated this way are scoped by _source chain_ into individual gossip topics.
    ///
    /// [`gossipsub`]: libp2p::gossipsub
    pub gossipsub: libp2p::gossipsub::Behaviour,
}

impl P2PBehavior {
    pub fn new(
        key: &libp2p::identity::Keypair,
        enable_mdns: bool,
        topic: &libp2p::gossipsub::IdentTopic,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(peer_id = %key.public().to_peer_id(), "🔭 Starting new p2p node");

        let ping = libp2p::ping::Behaviour::new(
            libp2p::ping::Config::new()
                .with_interval(std::time::Duration::from_secs(15))
                .with_timeout(std::time::Duration::from_secs(10)),
        );

        let limits = libp2p::connection_limits::Behaviour::new(
            libp2p::connection_limits::ConnectionLimits::default()
                .with_max_established_per_peer(Some(8)),
        );

        let identify = libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
            super::protocols::IDENTIFY.to_string(),
            key.public(),
        ));

        let mdns = if enable_mdns {
            tracing::info!("🔍 mDNS local peer discovery enabled");
            libp2p::swarm::behaviour::toggle::Toggle::from(Some(
                libp2p::mdns::tokio::Behaviour::new(
                    libp2p::mdns::Config::default(),
                    key.public().to_peer_id(),
                )?,
            ))
        } else {
            tracing::info!("🔇 mDNS local peer discovery disabled");
            libp2p::swarm::behaviour::toggle::Toggle::from(None)
        };

        let kad = libp2p::kad::Behaviour::with_config(
            key.public().to_peer_id(),
            libp2p::kad::store::MemoryStore::new(key.public().to_peer_id()),
            libp2p::kad::Config::new(super::protocols::KADEMLIA),
        );

        let mut gossipsub = libp2p::gossipsub::Behaviour::new(
            libp2p::gossipsub::MessageAuthenticity::Signed(key.clone()),
            libp2p::gossipsub::ConfigBuilder::default()
                .heartbeat_interval(std::time::Duration::from_secs(10))
                .validation_mode(libp2p::gossipsub::ValidationMode::Strict)
                .validate_messages()
                .build()?,
        )?;

        // Peer scoring: without it, `report_message_validation_result(.., Reject)` only drops
        // the individual message — a peer can flood invalid votes forever at no reputation
        // cost. With scoring active, repeat offenders accumulate a P4 (invalid message)
        // penalty and are progressively gossip-suppressed, then graylisted, then pruned from
        // the mesh (see `peer_score_params` for the tuning rationale).
        gossipsub.with_peer_score(
            peer_score_params(topic),
            libp2p::gossipsub::PeerScoreThresholds::default(),
        )?;

        Ok(Self {
            ping,
            limits,
            identify,
            mdns,
            kad,
            gossipsub,
        })
    }
}

/// Peer-scoring parameters for the vote topic.
///
/// Vote traffic is naturally *bursty* — every attestor publishes once per attestation interval,
/// then the topic goes quiet — so the mesh-delivery-rate components (P3/P3b), which penalize
/// peers for *not* delivering during a window, are disabled: they would punish honest attestors
/// for the idle stretches between intervals. Scoring instead leans on the components that only
/// malicious behavior can trip:
///
/// * **P4 (invalid messages)** — the primary defense (weight −10, squared counter): one
///   rejected vote suppresses gossip from the peer (default gossip threshold −10), three in
///   short succession graylist it (−90 < −80). The counter decays over ~10 minutes, so a
///   one-off glitch is forgiven while a sustained flooder stays penalized.
/// * **P7 (behavioral penalties)** and slow-peer penalties keep their library defaults.
///
/// Modest positive P1/P2 weights let long-lived, honestly-publishing peers build up a small
/// score buffer so a single stray invalid message doesn't immediately gossip-suppress a good
/// peer. Loopback is whitelisted from the IP-colocation penalty so local/zombienet clusters
/// (many peers on 127.0.0.1) don't self-penalize.
fn peer_score_params(topic: &libp2p::gossipsub::IdentTopic) -> libp2p::gossipsub::PeerScoreParams {
    let topic_params = libp2p::gossipsub::TopicScoreParams {
        topic_weight: 1.0,
        // P1: time in mesh — small, capped positive credit for stable peers.
        time_in_mesh_weight: 0.1,
        time_in_mesh_quantum: std::time::Duration::from_secs(1),
        time_in_mesh_cap: 300.0,
        // P2: first message deliveries — credit for actually contributing votes.
        first_message_deliveries_weight: 0.5,
        first_message_deliveries_decay: libp2p::gossipsub::score_parameter_decay(
            std::time::Duration::from_secs(600),
        ),
        first_message_deliveries_cap: 100.0,
        // P3/P3b: mesh delivery rate — disabled (bursty topic, see above).
        mesh_message_deliveries_weight: 0.0,
        mesh_message_deliveries_decay: 0.5,
        mesh_message_deliveries_cap: 100.0,
        mesh_message_deliveries_threshold: 1.0,
        mesh_message_deliveries_window: std::time::Duration::from_millis(10),
        mesh_message_deliveries_activation: std::time::Duration::from_secs(5),
        mesh_failure_penalty_weight: 0.0,
        mesh_failure_penalty_decay: 0.5,
        // P4: invalid messages — the piece issue-reporting `Reject` feeds into.
        invalid_message_deliveries_weight: -10.0,
        invalid_message_deliveries_decay: libp2p::gossipsub::score_parameter_decay(
            std::time::Duration::from_secs(600),
        ),
    };

    let mut params = libp2p::gossipsub::PeerScoreParams {
        // We never assign application-specific scores.
        app_specific_weight: 0.0,
        ..Default::default()
    };
    params.topics.insert(topic.hash(), topic_params);
    params
        .ip_colocation_factor_whitelist
        .insert(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    params
}
