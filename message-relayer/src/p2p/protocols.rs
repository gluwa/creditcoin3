//! Protocol identifiers used by the relayer's libp2p stack.
//!
//! The relayer participates in the same gossipsub *mesh* as attesters but identifies itself
//! under its own namespace so peers can tell relayers and attesters apart in identify / kad
//! protocol negotiation.

use libp2p::StreamProtocol;

pub const IDENTIFY: &str = "/gluwa/relayer-id/1.0.0";
pub const KADEMLIA: StreamProtocol = StreamProtocol::new("/gluwa/relayer-kad/1.0.0");

/// Build the gossipsub topic the relayer subscribes to for a given USC chain_key. Must match
/// the topic attesters publish on (PoC §6.1).
#[must_use]
pub fn message_votes_topic(chain_key: u64) -> String {
    format!("{chain_key}/message-votes/v1")
}
