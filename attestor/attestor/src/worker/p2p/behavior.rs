use crate::prelude::*;

#[derive(libp2p::swarm::NetworkBehaviour)]
pub(crate) struct P2PBehavior {
    /// [`Ping`] is used for peer reputation by disconnecting peers which repeatedly fail to
    /// respond to ping requests.
    ///
    /// [`Ping`]: libp2p::ping
    pub ping: libp2p::ping::Behaviour,

    pub limits: libp2p::connection_limits::Behaviour,

    /// [`Identify`] is used for identification between peers. This is required as other protocols
    /// in this behavior do not implement identification of their own.
    ///
    /// [`Identify`]: libp2p::identify
    pub identify: libp2p::identify::Behaviour,

    /// [`mDNS`] is used for _local_ node discovery. This is handy for testing or setting up clusters
    /// under the same local network but doesn't solve for global peer discovery.
    ///
    /// [`mDNS`]: libp2p::mdns
    pub mdns: libp2p::mdns::tokio::Behaviour,

    /// [`kademlia`] is used for _global_ peer discovery. We use kademlia instead of [`rendezvous`]
    /// for its resilience to centralized points of failure as well as its in-build peer discovery.
    ///
    /// [`kademlia`]: libp2p::kad
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
    pub fn new(key: &libp2p::identity::Keypair) -> common::types::Result<Self> {
        tracing::info!(peer_id = %key.public().to_peer_id(), "🔭 Starting new p2p node");

        let ping = libp2p::ping::Behaviour::new(
            libp2p::ping::Config::new()
                .with_interval(std::time::Duration::from_secs(60))
                .with_timeout(std::time::Duration::from_secs(30)),
        );

        let limits = libp2p::connection_limits::Behaviour::new(
            libp2p::connection_limits::ConnectionLimits::default()
                .with_max_established_per_peer(Some(8)),
        );

        let identify = libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
            super::protocols::IDENTIFY.to_string(),
            key.public(),
        ));

        let mdns = libp2p::mdns::tokio::Behaviour::new(
            libp2p::mdns::Config::default(),
            key.public().to_peer_id(),
        )?;

        let kad = libp2p::kad::Behaviour::with_config(
            key.public().to_peer_id(),
            libp2p::kad::store::MemoryStore::new(key.public().to_peer_id()),
            libp2p::kad::Config::new(super::protocols::KADEMLIA),
        );

        let gossipsub = libp2p::gossipsub::Behaviour::new(
            libp2p::gossipsub::MessageAuthenticity::Signed(key.clone()),
            libp2p::gossipsub::ConfigBuilder::default()
                .heartbeat_interval(std::time::Duration::from_secs(10))
                .validation_mode(libp2p::gossipsub::ValidationMode::Strict)
                .validate_messages()
                .build()?,
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
