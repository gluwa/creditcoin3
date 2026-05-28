//! A deliberately unauthorized p2p peer, for end-to-end testing of attestor peer authorization.
//!
//! This stands in for a hostile node: it is **not** registered in the on-chain attestor set and
//! holds no valid BLS key, yet it tries to join the attestor p2p network and flood the gossip
//! topic. It speaks the gluwa protocols (`/gluwa/id`, gossipsub) directly with hardcoded protocol
//! IDs — exactly what an attacker would do — rather than reusing the attestor crate internals.
//!
//! It intentionally does **not** implement the `/gluwa/auth` handshake. The authorized attestor
//! proactively challenges it on connection; because this peer cannot speak the protocol, that
//! challenge fails immediately and the attestor blocklists it (a silent peer that completes the
//! transport but ignores the challenge is instead caught by the auth timeout sweep). Run it against
//! a live network (e.g. one spun up with the `zombienet` helper) and watch the authorized
//! attestor's logs:
//!
//! ```text
//! ⛔ Peer authorization request failed   (error: UnsupportedProtocols)
//! ⛔ Blocklisting unauthorized peer
//! ```
//!
//! # Usage
//!
//! ```text
//! cargo run -p attestor --example rogue_peer -- \
//!     --target /ip4/127.0.0.1/tcp/9000/p2p/<ATTESTOR_PEER_ID> \
//!     [--chain-key 2] [--flood]
//! ```
//!
//! The target multiaddr (including `/p2p/<peer-id>`) can be copied from an attestor's startup log
//! line `🔭 Starting new p2p node` (peer id) together with its listen port.

use futures::StreamExt as _;

// The attestor's protocol identifiers, hardcoded as a hostile peer would. Kept in sync with
// `attestor/src/worker/p2p/protocols.rs` and the gossip topic in `worker::p2p`.
const IDENTIFY_PROTOCOL: &str = "/gluwa/id/1.0.0";

#[derive(libp2p::swarm::NetworkBehaviour)]
struct RogueBehaviour {
    identify: libp2p::identify::Behaviour,
    gossipsub: libp2p::gossipsub::Behaviour,
}

struct Args {
    target: libp2p::Multiaddr,
    chain_key: u64,
    flood: bool,
}

fn parse_args() -> Args {
    let mut target = None;
    let mut chain_key = 2u64;
    let mut flood = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target" => {
                target = Some(
                    args.next()
                        .expect("--target requires a multiaddr")
                        .parse()
                        .expect("invalid multiaddr"),
                );
            }
            "--chain-key" => {
                chain_key = args
                    .next()
                    .expect("--chain-key requires a value")
                    .parse()
                    .expect("invalid chain key");
            }
            "--flood" => flood = true,
            other => panic!("unknown argument: {other}"),
        }
    }

    Args {
        target: target.expect("missing required --target <multiaddr>"),
        chain_key,
        flood,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();

    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            libp2p::tcp::Config::default(),
            libp2p::noise::Config::new,
            libp2p::yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|key| {
            let identify = libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
                IDENTIFY_PROTOCOL.to_string(),
                key.public(),
            ));

            let gossipsub = libp2p::gossipsub::Behaviour::new(
                libp2p::gossipsub::MessageAuthenticity::Signed(key.clone()),
                libp2p::gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(std::time::Duration::from_secs(10))
                    .validation_mode(libp2p::gossipsub::ValidationMode::Strict)
                    .validate_messages()
                    .build()
                    .expect("valid gossipsub config"),
            )
            .expect("valid gossipsub behaviour");

            RogueBehaviour {
                identify,
                gossipsub,
            }
        })?
        .build();

    let topic = libp2p::gossipsub::IdentTopic::new(format!("{}/attest", args.chain_key));
    swarm.behaviour_mut().gossipsub.subscribe(&topic)?;

    println!("🏴‍☠️ Rogue peer id: {}", swarm.local_peer_id());
    println!("🎯 Dialing target: {}", args.target);
    println!("📫 Subscribed to gossip topic: {topic}");
    if args.flood {
        println!("🌊 Flooding mode: will publish junk to the gossip topic");
    }
    println!(
        "ℹ️  This peer cannot complete /gluwa/auth — expect to be blocklisted near-instantly.\n"
    );

    swarm.dial(args.target.clone())?;

    let mut flood_interval = tokio::time::interval(std::time::Duration::from_secs(1));
    let mut junk_counter: u64 = 0;

    loop {
        tokio::select! {
            _ = flood_interval.tick(), if args.flood => {
                junk_counter += 1;
                let junk = format!("rogue-junk-{junk_counter}").into_bytes();
                match swarm.behaviour_mut().gossipsub.publish(topic.hash(), junk) {
                    Ok(_) => println!("🌊 Published junk message #{junk_counter} to the network"),
                    Err(err) => println!("🌊 Publish #{junk_counter} failed (not yet meshed?): {err}"),
                }
            }
            event = swarm.select_next_some() => match event {
                libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("🔗 Connected to attestor {peer_id}");
                }
                libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                    println!("⛓️‍💥 Disconnected from attestor {peer_id} (cause: {cause:?})");
                    println!("   → this is the attestor blocklisting us for failing /gluwa/auth.");
                }
                libp2p::swarm::SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    println!("⛔ Connection refused by {peer_id:?}: {error}");
                    println!("   → once blocklisted, the attestor denies all reconnect attempts.");
                }
                libp2p::swarm::SwarmEvent::Behaviour(RogueBehaviourEvent::Identify(
                    libp2p::identify::Event::Received { peer_id, .. },
                )) => {
                    println!("🛰️  Identified attestor {peer_id}");
                }
                _ => {}
            }
        }
    }
}
