//! Inbound connection admission that keeps capacity for known committee peers.
//!
//! Inbound capacity is split: *reserved* peers (configured boot peers and peers bound to a
//! BLS-verified attestor vote) are always admitted, while every other peer shares a pool of
//! [`MAX_UNRESERVED_INCOMING`] connections. This complements [`connection_limits`], which applies
//! one cap to every peer.
//!
//! [`connection_limits`]: libp2p::connection_limits

use std::{
    collections::{HashMap, HashSet},
    task::{Context, Poll},
};

use libp2p::{
    core::{transport::PortUse, Endpoint},
    swarm::{
        behaviour::{ConnectionClosed, ConnectionEstablished},
        dummy, ConnectionDenied, ConnectionId, FromSwarm, NetworkBehaviour, THandler,
        THandlerInEvent, THandlerOutEvent, ToSwarm,
    },
    Multiaddr, PeerId,
};

/// Inbound connections from non-reserved peers share this many slots.
pub(crate) const MAX_UNRESERVED_INCOMING: usize = 64;

pub(crate) struct Behaviour {
    max_unreserved_incoming: usize,
    reserved: HashSet<PeerId>,
    /// Established inbound connections from non-reserved peers.
    unreserved_incoming: HashMap<ConnectionId, PeerId>,
}

impl Behaviour {
    pub(crate) fn new(max_unreserved_incoming: usize) -> Self {
        Self {
            max_unreserved_incoming,
            reserved: HashSet::new(),
            unreserved_incoming: HashMap::new(),
        }
    }

    /// Always admit `peer`; its existing connections stop counting against the shared pool.
    pub(crate) fn reserve(&mut self, peer: PeerId) {
        if self.reserved.insert(peer) {
            self.unreserved_incoming.retain(|_, p| *p != peer);
        }
    }

    /// Treat `peer` like any other peer again (e.g. its attestor was chilled or kicked).
    pub(crate) fn unreserve(&mut self, peer: &PeerId) {
        self.reserved.remove(peer);
    }

    fn admit(&self, peer: &PeerId) -> Result<(), ConnectionDenied> {
        if self.reserved.contains(peer)
            || self.unreserved_incoming.len() < self.max_unreserved_incoming
        {
            Ok(())
        } else {
            Err(ConnectionDenied::new(UnreservedCapacityExceeded {
                limit: self.max_unreserved_incoming,
            }))
        }
    }
}

#[derive(Debug)]
struct UnreservedCapacityExceeded {
    limit: usize,
}

impl std::fmt::Display for UnreservedCapacityExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "inbound capacity for unreserved peers reached ({})",
            self.limit
        )
    }
}

impl std::error::Error for UnreservedCapacityExceeded {}

impl NetworkBehaviour for Behaviour {
    type ConnectionHandler = dummy::ConnectionHandler;
    type ToSwarm = std::convert::Infallible;

    fn handle_established_inbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        peer: PeerId,
        _local_addr: &Multiaddr,
        _remote_addr: &Multiaddr,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        self.admit(&peer)?;
        Ok(dummy::ConnectionHandler)
    }

    fn handle_established_outbound_connection(
        &mut self,
        _connection_id: ConnectionId,
        _peer: PeerId,
        _addr: &Multiaddr,
        _role_override: Endpoint,
        _port_use: PortUse,
    ) -> Result<THandler<Self>, ConnectionDenied> {
        Ok(dummy::ConnectionHandler)
    }

    fn on_swarm_event(&mut self, event: FromSwarm) {
        match event {
            FromSwarm::ConnectionEstablished(ConnectionEstablished {
                peer_id,
                connection_id,
                endpoint,
                ..
            }) => {
                if endpoint.is_listener() && !self.reserved.contains(&peer_id) {
                    self.unreserved_incoming.insert(connection_id, peer_id);
                }
            }
            FromSwarm::ConnectionClosed(ConnectionClosed { connection_id, .. }) => {
                self.unreserved_incoming.remove(&connection_id);
            }
            _ => {}
        }
    }

    fn on_connection_handler_event(
        &mut self,
        _peer_id: PeerId,
        _connection_id: ConnectionId,
        event: THandlerOutEvent<Self>,
    ) {
        match event {}
    }

    fn poll(&mut self, _: &mut Context<'_>) -> Poll<ToSwarm<Self::ToSwarm, THandlerInEvent<Self>>> {
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(behaviour: &mut Behaviour, n: usize) {
        for _ in 0..n {
            behaviour
                .unreserved_incoming
                .insert(ConnectionId::new_unchecked(rand_id()), PeerId::random());
        }
    }

    fn rand_id() -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    #[test]
    fn unreserved_peers_share_a_bounded_pool() {
        let mut behaviour = Behaviour::new(2);
        assert!(behaviour.admit(&PeerId::random()).is_ok());
        fill(&mut behaviour, 2);
        assert!(behaviour.admit(&PeerId::random()).is_err());
    }

    #[test]
    fn reserved_peers_are_admitted_when_the_pool_is_full() {
        let mut behaviour = Behaviour::new(2);
        fill(&mut behaviour, 2);
        let member = PeerId::random();
        behaviour.reserve(member);
        assert!(behaviour.admit(&member).is_ok());

        behaviour.unreserve(&member);
        assert!(behaviour.admit(&member).is_err());
    }

    #[test]
    fn reserving_a_peer_frees_its_unreserved_slots() {
        let mut behaviour = Behaviour::new(1);
        let member = PeerId::random();
        behaviour
            .unreserved_incoming
            .insert(ConnectionId::new_unchecked(rand_id()), member);
        assert!(behaviour.admit(&PeerId::random()).is_err());

        behaviour.reserve(member);
        assert!(behaviour.admit(&PeerId::random()).is_ok());
    }
}
