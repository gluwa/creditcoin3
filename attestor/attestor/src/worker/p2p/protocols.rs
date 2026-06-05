pub(super) const IDENTIFY: &str = "/gluwa/id/1.0.0";
pub(super) const KADEMLIA: libp2p::StreamProtocol = libp2p::StreamProtocol::new("/gluwa/kad/1.0.0");

/// Peer authorization handshake. On connection, peers exchange a [`PeerAuth`] proving they control
/// the BLS private key of an attestor in the on-chain active set, binding that identity to their
/// libp2p [`PeerId`]. Peers which fail this handshake are disconnected and blocklisted.
///
/// [`PeerAuth`]: super::auth::PeerAuth
/// [`PeerId`]: libp2p::PeerId
pub(super) const AUTH: libp2p::StreamProtocol = libp2p::StreamProtocol::new("/gluwa/auth/1.0.0");
