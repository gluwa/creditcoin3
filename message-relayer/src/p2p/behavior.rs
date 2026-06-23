//! libp2p `NetworkBehaviour` for the relayer.
//!
//! Mirrors `attestor/attestor/src/worker/p2p/behavior.rs` with two intentional differences:
//!
//!  * relayer-namespaced identify / kad protocol ids ([`super::protocols`]) so peers can tell
//!    relayers and attestors apart on the wire,
//!  * gossipsub uses **permissive** validation mode so we can subscribe without participating
//!    in attestor-only signing: relayers do not need to sign the messages they republish.

use libp2p::swarm::NetworkBehaviour;

use super::protocols;

#[derive(NetworkBehaviour)]
pub struct RelayerBehavior {
    pub ping: libp2p::ping::Behaviour,
    pub limits: libp2p::connection_limits::Behaviour,
    pub identify: libp2p::identify::Behaviour,
    pub mdns: libp2p::swarm::behaviour::toggle::Toggle<libp2p::mdns::tokio::Behaviour>,
    pub kad: libp2p::kad::Behaviour<libp2p::kad::store::MemoryStore>,
    pub gossipsub: libp2p::gossipsub::Behaviour,
}

impl RelayerBehavior {
    pub fn new(
        key: &libp2p::identity::Keypair,
        enable_mdns: bool,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(peer_id = %key.public().to_peer_id(), "🔭 Starting relayer p2p node");

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
            protocols::IDENTIFY.to_string(),
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
            libp2p::kad::Config::new(protocols::KADEMLIA),
        );

        let gossipsub = libp2p::gossipsub::Behaviour::new(
            libp2p::gossipsub::MessageAuthenticity::Signed(key.clone()),
            libp2p::gossipsub::ConfigBuilder::default()
                .heartbeat_interval(std::time::Duration::from_secs(10))
                // Strict gossipsub-level validation matches the attestor mesh; the relayer
                // additionally validates the *envelope* before counting votes (PoC §6.2).
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
