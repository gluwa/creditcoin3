//! Protocol identifiers used by the relayer's libp2p stack.
//!
//! The relayer participates in the same gossipsub *mesh* as attestors but identifies itself
//! under its own namespace so peers can tell relayers and attestors apart in identify / kad
//! protocol negotiation.

use libp2p::StreamProtocol;

pub const IDENTIFY: &str = "/gluwa/relayer-id/1.0.0";
pub const KADEMLIA: StreamProtocol = StreamProtocol::new("/gluwa/relayer-kad/1.0.0");

/// The gossipsub topic the relayer subscribes to for a given USC chain_key. Defined in the shared
/// [`write_ability`] crate so it always matches the topic attestors publish on (PoC §6.1).
pub use write_ability::protocol::message_votes_topic;
