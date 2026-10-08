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
    /// Build the behaviour.
    ///
    /// Between them, `credited_topics` and `penalty_only_topics` must list EVERY topic this node will
    /// subscribe to. Peer scoring is per-topic in libp2p-gossipsub: `PeerScoreParams::topics` is a map,
    /// and `mark_invalid_message_delivery` silently no-ops for a topic with no entry in it (it looks the
    /// topic up and returns early when absent). A subscribed-but-unscored topic therefore makes
    /// `report_message_validation_result(.., Reject)` a pure no-op on that topic — the offending peer
    /// accrues no P4 penalty, is never gossip-suppressed and is never graylisted — so a flooder can pump
    /// forged frames at zero reputation cost, each costing us a decode and (on the vote topic) a
    /// secp256k1 recovery, inline in the same task that serves block-attestation votes.
    ///
    /// The split matters because per-topic scores are SUMMED into one peer score that the thresholds are
    /// applied to, so positive credit earned on one topic offsets penalties earned on another:
    ///
    /// * `credited_topics` — topics where an `Accept` means the frame passed real validation, so P2
    ///   (first-message-delivery) credit is earned honestly. The block-attestation and message-vote
    ///   topics qualify: both fully decode the frame, and the vote topic also recovers a secp256k1
    ///   signature, before deciding.
    /// * `penalty_only_topics` — topics that `Accept` frames without fully validating them. These get the
    ///   same negative P4 weight and no positive weights at all, because crediting P1/P2 would let a peer
    ///   farm positive score cheaply and use that buffer to absorb P4 penalties earned on the topics that
    ///   DO validate. Two topics qualify:
    ///   - attestor-set-update: only checks a frame-size bound before accepting and propagating.
    ///   - reobservation: accepts as soon as the frame decodes and its `chain_key` matches.
    ///     `ReobservationRequest` carries no signature — just `chain_key`, `message_id`, `tx_hash` and
    ///     `block_height` — so well-formed requests naming arbitrary message ids are free to produce.
    ///     The RPC check runs later in the write-ability worker and never feeds back into
    ///     `report_message_validation_result`, so it cannot make the `Accept` meaningful.
    pub fn new(
        key: &libp2p::identity::Keypair,
        enable_mdns: bool,
        credited_topics: &[&libp2p::gossipsub::IdentTopic],
        penalty_only_topics: &[&libp2p::gossipsub::IdentTopic],
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(peer_id = %key.public().to_peer_id(), "🔭 Starting new p2p node");

        let ping = libp2p::ping::Behaviour::new(
            libp2p::ping::Config::new()
                .with_interval(std::time::Duration::from_secs(15))
                .with_timeout(std::time::Duration::from_secs(10)),
        );

        // Per-peer cap bounds a single peer's connections; the global caps bound the *total*
        // established connections so a Sybil attacker opening connections from many fresh peer
        // ids can't exhaust file descriptors / memory / gossip-processing capacity. Attestor
        // networks are committee-sized (tens of peers), so these limits are generous for
        // legitimate topologies while still bounding resource pressure.
        let limits = libp2p::connection_limits::Behaviour::new(
            libp2p::connection_limits::ConnectionLimits::default()
                .with_max_established_per_peer(Some(8))
                .with_max_established(Some(512))
                .with_max_established_incoming(Some(256)),
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
        //
        // Every subscribed topic must be listed here — see the note on `new`. Scoring params can only be
        // installed once, so the caller passes all topics up front rather than adding them as it
        // subscribes.
        debug_assert!(
            !credited_topics.is_empty(),
            "gossipsub peer scoring needs at least one topic; an unscored topic makes Reject a no-op"
        );
        gossipsub.with_peer_score(
            peer_score_params(credited_topics, penalty_only_topics),
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

/// Peer-scoring parameters, applied identically to every topic this node subscribes to.
///
/// One shared tuning fits all of them: block-attestation votes, write-ability message votes,
/// reobservation requests and attestor-set-update votes are all bursty, all originate from a
/// committee-sized peer set, and all reserve `Reject` for *provable* invalidity (undecodable frame,
/// wrong chain key, forged signature) — never for state-dependent verdicts, which use `Ignore` so an
/// honest peer whose view merely lags is not penalised. That is what makes a shared, aggressive P4
/// weight safe here.
///
/// Vote traffic is naturally *bursty* — every attestor publishes once per attestation interval,
/// then the topic goes quiet — so the mesh-delivery-rate components (P3/P3b), which penalize
/// peers for *not* delivering during a window, are disabled: they would punish honest attestors
/// for the idle stretches between intervals. Scoring instead leans on the components that only
/// malicious behavior can trip:
///
/// * **P4 (invalid messages)** — the primary defense (weight −10, squared counter): from a zero
///   baseline one rejected vote suppresses gossip from the peer (default gossip threshold −10) and
///   three in short succession graylist it (−90 < −80). The counter decays over ~10 minutes, so a
///   one-off glitch is forgiven while a sustained flooder stays penalized. Those exact counts assume
///   no positive buffer: per-topic scores are summed, so a long-lived honest peer that has banked P1/P2
///   credit absorbs one or two more rejects first — deliberate, and the reason topics that `Accept`
///   without validating get no positive weights at all (see `P2PBehavior::new`).
/// * **P7 (behavioral penalties)** and slow-peer penalties keep their library defaults.
///
/// Modest positive P1/P2 weights let long-lived, honestly-publishing peers build up a small
/// score buffer so a single stray invalid message doesn't immediately gossip-suppress a good
/// peer. Loopback is whitelisted from the IP-colocation penalty so local/zombienet clusters
/// (many peers on 127.0.0.1) don't self-penalize.
fn peer_score_params(
    credited_topics: &[&libp2p::gossipsub::IdentTopic],
    penalty_only_topics: &[&libp2p::gossipsub::IdentTopic],
) -> libp2p::gossipsub::PeerScoreParams {
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
    for topic in credited_topics {
        params.topics.insert(topic.hash(), topic_params.clone());
    }
    // Same P4 penalty, zero positive credit — so `Reject` bites on these topics without their `Accept`s
    // becoming a score faucet that offsets penalties elsewhere (see the note on `new`).
    let penalty_only = libp2p::gossipsub::TopicScoreParams {
        time_in_mesh_weight: 0.0,
        first_message_deliveries_weight: 0.0,
        ..topic_params.clone()
    };
    for topic in penalty_only_topics {
        params.topics.insert(topic.hash(), penalty_only.clone());
    }
    params
        .ip_colocation_factor_whitelist
        .insert(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the audit finding: the three write-ability topics were subscribed but left
    /// out of `PeerScoreParams::topics`, which makes `Reject` on them a silent no-op. Any topic the
    /// gossip task subscribes to must be scored, so assert the mapping is total and that P4 actually
    /// bites on each one.
    #[test]
    fn every_passed_topic_gets_score_params() {
        let attest = libp2p::gossipsub::IdentTopic::new("7/attest");
        let votes =
            libp2p::gossipsub::IdentTopic::new(write_ability::protocol::message_votes_topic(7));
        let reobs =
            libp2p::gossipsub::IdentTopic::new(write_ability::protocol::reobservation_topic(7));
        let set_update = libp2p::gossipsub::IdentTopic::new(
            write_ability::protocol::attestor_set_update_topic(7),
        );

        // `reobs` belongs here, not in `credited`: `handle_reobservation_request` Accepts on decode +
        // chain-key match, and `ReobservationRequest` is unsigned, so the Accept attests to nothing an
        // attacker cannot trivially produce.
        let credited = [&attest, &votes];
        let penalty_only = [&set_update, &reobs];
        let params = peer_score_params(&credited, &penalty_only);

        assert_eq!(params.topics.len(), credited.len() + penalty_only.len());

        // Every subscribed topic must penalize Reject, or the flood is free.
        for topic in credited.iter().chain(penalty_only.iter()) {
            let entry = params.topics.get(&topic.hash()).unwrap_or_else(|| {
                panic!("{topic} has no TopicScoreParams — Reject on it would be a no-op")
            });
            assert!(
                entry.invalid_message_deliveries_weight < 0.0,
                "{topic} must carry a negative P4 weight for Reject to cost the sender anything"
            );
        }

        // Topics that Accept without validating must grant no positive credit: per-topic scores are
        // summed, so credit banked on a cheaply-spammable topic would offset P4 earned on the topics
        // that do validate.
        for topic in penalty_only {
            let entry = &params.topics[&topic.hash()];
            assert_eq!(
                entry.time_in_mesh_weight, 0.0,
                "{topic} must not grant P1 credit"
            );
            assert_eq!(
                entry.first_message_deliveries_weight, 0.0,
                "{topic} must not grant P2 credit"
            );
        }

        // ...whereas topics whose Accept means "passed validation" keep the positive buffer that stops a
        // single stray invalid message from suppressing a good peer.
        for topic in credited {
            let entry = &params.topics[&topic.hash()];
            assert!(entry.first_message_deliveries_weight > 0.0);
        }
    }
}
