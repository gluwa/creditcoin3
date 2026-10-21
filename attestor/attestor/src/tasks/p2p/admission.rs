//! Inbound connection admission that keeps capacity for known committee peers.
//!
//! Inbound capacity is split: *reserved* peers (configured boot/trusted committee peers and peers
//! bound to a BLS-verified attestor vote) are admitted by this behaviour, while every other peer
//! shares a pool of [`MAX_UNRESERVED_INCOMING`] connections. This complements
//! [`connection_limits`], which applies one cap to every peer.
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
    /// Configured boot/trusted peers: reserved for the node's lifetime, never unreserved.
    pinned: HashSet<PeerId>,
    /// Manual rollout fallback, replaced when finalized registration becomes mandatory.
    configured: HashSet<PeerId>,
    /// Finalized on-chain committee identities; replaced atomically on membership/rotation.
    committee: HashSet<PeerId>,
    /// Established inbound connections. Whether one counts against the shared pool is decided
    /// at admission time, so reserving or unreserving a peer moves its live connections too.
    incoming: HashMap<ConnectionId, PeerId>,
}

impl Behaviour {
    pub(crate) fn new(max_unreserved_incoming: usize) -> Self {
        Self {
            max_unreserved_incoming,
            reserved: HashSet::new(),
            pinned: HashSet::new(),
            configured: HashSet::new(),
            committee: HashSet::new(),
            incoming: HashMap::new(),
        }
    }

    /// Always admit `peer`; its existing connections stop counting against the shared pool.
    pub(crate) fn reserve(&mut self, peer: PeerId) {
        self.reserved.insert(peer);
    }

    /// Always admit `peer` for the node's lifetime; [`Self::unreserve`] leaves it reserved.
    pub(crate) fn pin(&mut self, peer: PeerId) {
        self.pinned.insert(peer);
        self.reserve(peer);
    }

    /// Release a learned vote binding, leaving independently configured/finalized trust intact.
    /// Membership eviction uses [`Self::revoke`] to clear those sources too.
    pub(crate) fn unreserve(&mut self, peer: &PeerId) {
        if !self.pinned.contains(peer) {
            self.reserved.remove(peer);
        }
    }

    /// Revoke a deactivated member's reservations from every dynamic source. Explicit boot
    /// trust remains independent of committee membership.
    pub(crate) fn revoke(&mut self, peer: &PeerId) {
        self.unreserve(peer);
        self.configured.remove(peer);
        self.committee.remove(peer);
    }

    pub(crate) fn set_configured_peers(&mut self, peers: impl IntoIterator<Item = PeerId>) {
        self.configured = peers.into_iter().collect();
    }

    pub(crate) fn set_committee_peers(
        &mut self,
        peers: impl IntoIterator<Item = PeerId>,
        registration_required: bool,
    ) {
        self.committee = peers.into_iter().collect();
        if registration_required {
            self.configured.clear();
            self.reserved.retain(|peer| self.pinned.contains(peer));
        }
    }

    pub(crate) fn set_learned_peers(&mut self, peers: impl IntoIterator<Item = PeerId>) {
        self.reserved = self.pinned.iter().copied().chain(peers).collect();
    }

    pub(crate) fn is_reserved(&self, peer: &PeerId) -> bool {
        self.reserved.contains(peer)
            || self.configured.contains(peer)
            || self.committee.contains(peer)
    }

    pub(crate) fn reserved_peers(&self) -> HashSet<PeerId> {
        self.reserved
            .iter()
            .chain(&self.configured)
            .chain(&self.committee)
            .copied()
            .collect()
    }

    fn unreserved_incoming(&self) -> usize {
        self.incoming
            .values()
            .filter(|p| !self.is_reserved(p))
            .count()
    }

    fn admit(&self, peer: &PeerId) -> Result<(), ConnectionDenied> {
        if self.is_reserved(peer) || self.unreserved_incoming() < self.max_unreserved_incoming {
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
                if endpoint.is_listener() {
                    self.incoming.insert(connection_id, peer_id);
                }
            }
            FromSwarm::ConnectionClosed(ConnectionClosed { connection_id, .. }) => {
                self.incoming.remove(&connection_id);
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

    #[test]
    fn finalized_committee_connects_before_any_vote_even_with_a_full_unknown_pool() {
        let mut behaviour = Behaviour::new(MAX_UNRESERVED_INCOMING);
        fill(&mut behaviour, MAX_UNRESERVED_INCOMING);
        let member = PeerId::random();
        behaviour.set_committee_peers([member], true);
        super::super::configure_bootstrap_admission(
            &mut behaviour,
            &HashSet::new(),
            &[],
            PeerId::random(),
            false,
            true,
            true,
        )
        .unwrap();
        assert!(behaviour.admit(&member).is_ok());
        assert!(behaviour.admit(&PeerId::random()).is_err());
    }

    #[test]
    fn finalized_rotation_and_chill_revoke_old_reservations() {
        let mut behaviour = Behaviour::new(1);
        fill(&mut behaviour, 1);
        let old = PeerId::random();
        let new = PeerId::random();
        behaviour.set_committee_peers([old], true);
        assert!(behaviour.admit(&old).is_ok());
        behaviour.set_committee_peers([new], true);
        assert!(behaviour.admit(&old).is_err());
        assert!(behaviour.admit(&new).is_ok());
        behaviour.set_committee_peers([], true);
        assert!(behaviour.admit(&new).is_err());
    }

    #[test]
    fn enforcement_replaces_manual_and_learned_fallback_but_keeps_boot_peers() {
        let mut behaviour = Behaviour::new(0);
        let (boot, configured, learned) = (PeerId::random(), PeerId::random(), PeerId::random());
        behaviour.pin(boot);
        behaviour.set_configured_peers([configured]);
        behaviour.reserve(learned);
        behaviour.set_committee_peers([], true);
        assert!(behaviour.admit(&boot).is_ok());
        assert!(behaviour.admit(&configured).is_err());
        assert!(behaviour.admit(&learned).is_err());
    }

    #[test]
    fn seed_reserves_configured_member_before_its_first_vote() {
        let mut behaviour = Behaviour::new(MAX_UNRESERVED_INCOMING);
        fill(&mut behaviour, MAX_UNRESERVED_INCOMING);
        let member = PeerId::random();
        let address: Multiaddr = "/ip4/127.0.0.1/tcp/9000".parse().unwrap();

        // No boot peers or learned vote bindings: exercise the seed's actual startup setup.
        super::super::configure_bootstrap_admission(
            &mut behaviour,
            &HashSet::new(),
            &[member],
            PeerId::random(),
            false,
            true,
            false,
        )
        .unwrap();

        assert!(behaviour
            .handle_established_inbound_connection(
                ConnectionId::new_unchecked(rand_id()),
                member,
                &address,
                &address,
            )
            .is_ok());
        assert!(behaviour.admit(&PeerId::random()).is_err());
        assert_eq!(behaviour.unreserved_incoming(), MAX_UNRESERVED_INCOMING);

        // Changing a learned binding does not erase configured trust.
        behaviour.unreserve(&member);
        assert!(behaviour.admit(&member).is_ok());
    }

    #[test]
    fn revocation_clears_all_dynamic_reservations_and_recounts_live_connections() {
        let mut behaviour = Behaviour::new(1);
        let member = PeerId::random();
        behaviour.reserve(member);
        behaviour.set_configured_peers([member]);
        behaviour.set_committee_peers([member], false);
        behaviour
            .incoming
            .insert(ConnectionId::new_unchecked(rand_id()), member);
        assert_eq!(behaviour.unreserved_incoming(), 0);
        behaviour.revoke(&member);
        assert!(!behaviour.is_reserved(&member));
        assert_eq!(behaviour.unreserved_incoming(), 1);
        assert!(behaviour.admit(&member).is_err());
    }

    #[test]
    fn revocation_preserves_explicit_boot_trust() {
        let mut behaviour = Behaviour::new(0);
        let boot = PeerId::random();
        behaviour.pin(boot);
        behaviour.set_configured_peers([boot]);
        behaviour.set_committee_peers([boot], false);
        behaviour.revoke(&boot);
        assert!(behaviour.admit(&boot).is_ok());
    }

    #[test]
    fn public_seed_without_trusted_remote_peers_is_rejected() {
        let local = PeerId::random();
        // Neither mDNS nor pinning our own identity provides trusted inbound access.
        for enable_mdns in [false, true] {
            for reserved in [vec![], vec![local]] {
                let error = super::super::configure_bootstrap_admission(
                    &mut Behaviour::new(2),
                    &HashSet::new(),
                    &reserved,
                    local,
                    enable_mdns,
                    true,
                    false,
                )
                .unwrap_err();
                assert!(error.to_string().contains("reserved-peers"));
            }
        }
    }

    #[test]
    fn configured_boot_peer_provides_a_trusted_bootstrap_route() {
        let mut behaviour = Behaviour::new(2);
        fill(&mut behaviour, 2);
        let boot = PeerId::random();
        super::super::configure_bootstrap_admission(
            &mut behaviour,
            &HashSet::from([boot]),
            &[],
            PeerId::random(),
            false,
            true,
            false,
        )
        .unwrap();
        assert!(behaviour.admit(&boot).is_ok());
        assert!(behaviour.admit(&PeerId::random()).is_err());
    }

    #[test]
    fn own_boot_peer_does_not_make_a_public_seed_safe() {
        let local = PeerId::random();
        assert!(super::super::configure_bootstrap_admission(
            &mut Behaviour::new(2),
            &HashSet::from([local]),
            &[],
            local,
            false,
            true,
            false,
        )
        .is_err());
    }

    #[test]
    fn reserved_peer_ids_do_not_replace_discovery_for_a_non_seed() {
        assert!(super::super::configure_bootstrap_admission(
            &mut Behaviour::new(2),
            &HashSet::new(),
            &[PeerId::random()],
            PeerId::random(),
            false,
            false,
            false,
        )
        .is_err());
    }

    #[test]
    fn local_mdns_network_still_bootstraps_without_configured_peers() {
        assert!(super::super::configure_bootstrap_admission(
            &mut Behaviour::new(2),
            &HashSet::new(),
            &[],
            PeerId::random(),
            true,
            false,
            false,
        )
        .is_ok());
    }

    fn fill(behaviour: &mut Behaviour, n: usize) {
        for _ in 0..n {
            behaviour
                .incoming
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
    fn unreserving_a_peer_returns_its_connections_to_the_pool() {
        let mut behaviour = Behaviour::new(1);
        let member = PeerId::random();
        behaviour.reserve(member);
        behaviour
            .incoming
            .insert(ConnectionId::new_unchecked(rand_id()), member);
        assert!(behaviour.admit(&PeerId::random()).is_ok());

        behaviour.unreserve(&member);
        assert!(behaviour.admit(&PeerId::random()).is_err());
    }

    #[test]
    fn pinned_peers_stay_reserved_after_unreserve() {
        let mut behaviour = Behaviour::new(2);
        fill(&mut behaviour, 2);
        let boot = PeerId::random();
        behaviour.pin(boot);

        behaviour.unreserve(&boot);
        assert!(behaviour.admit(&boot).is_ok());
    }

    #[test]
    fn reserving_a_peer_frees_its_unreserved_slots() {
        let mut behaviour = Behaviour::new(1);
        let member = PeerId::random();
        behaviour
            .incoming
            .insert(ConnectionId::new_unchecked(rand_id()), member);
        assert!(behaviour.admit(&PeerId::random()).is_err());

        behaviour.reserve(member);
        assert!(behaviour.admit(&PeerId::random()).is_ok());
    }
}
