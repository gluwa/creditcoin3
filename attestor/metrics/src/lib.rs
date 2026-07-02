//! Attestor [prometheus] metrics, see [`Metrics`] for a list of available metrics.
//!
//! [prometheus]:  prometheus_client

#[derive(builder::Builder)]
pub struct Config {
    name: String,
    address: cc_client::AccountId32,
    peer_id: libp2p::PeerId,
    chain_key: attestor_primitives::ChainKey,

    start_height: attestor_primitives::Height,
    start_attestation: Option<stream::util::AttestationInfo>,
    genesis: attestor_primitives::Height,

    attestation_latest_eth: attestor_primitives::Height,
    attestation_interval: std::num::NonZero<attestor_primitives::Height>,
}

/// Global atomic metrics store.
///
/// # Metrics
///
/// - [hardware]: basic hardware metrics.
/// - [production]: keeps track of attestation production relative to the execution chain.
/// - [lag]: monitors an attestor’s advancement relative to the source chain and the execution chain.
/// - [delay]: aggregates the elapsed time throughout an attestation’s life cycle.
/// - [p2p]: monitors p2p network health.
/// - [errors]: counts failed state.
///
/// [hardware]: Store::metrics_hardware
/// [production]: Store::metrics_production
/// [lag]: Store::metrics_lag
/// [delay]: Store::metrics_delay
/// [p2p]: Store::metrics_p2p
/// [errors]: Store::metrics_error
#[derive(Debug, Clone)]
pub struct Metrics(std::sync::Arc<Store>);

#[derive(Debug)]
struct Store {
    registry: prometheus_client::registry::Registry,

    /// Basic hardware metrics.
    ///
    /// - _CPU usage_ (via [`global_cpu_usage`])
    /// - _RAM usage_ (via [`used_memory`])
    ///
    /// See [`update_hardware`] for implementation details.
    ///
    /// [`global_cpu_usage`]: sysinfo::System::global_cpu_usage
    /// [`used_memory`]: sysinfo::System::used_memory
    /// [`update_hardware`]: Self::update_hardware
    pub metrics_hardware: prometheus_client::metrics::family::Family<
        labels::LabelHardware,
        prometheus_client::metrics::gauge::Gauge<f64, std::sync::atomic::AtomicU64>,
    >,

    /// Metrics which keep track of attestation production relative to the execution chain.
    ///
    /// - _Latest locally produced attestation height_ ([`Gauge`])
    /// - _Latest finalized attestation height_ ([`Gauge`])
    ///
    /// Finalization data is already encapsulated by the [lag] metrics, so these are more for
    /// debugging and local observability.
    ///
    /// See [`set_attestation_local`] and [`set_attestation_finalized`] for implementation details.
    ///
    /// [`Gauge`]: prometheus_client::metrics::gauge::Gauge
    /// [lag]: Self::metrics_lag
    /// [`set_attestation_local`]: Self::set_attestation_local
    /// [`set_attestation_finalized`]: Self::set_attestation_finalized
    pub metrics_production: prometheus_client::metrics::family::Family<
        labels::LabelAttestationProgress,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

    /// Metrics which keep track of an attestor’s advancement relative to the source chain and the
    /// execution chain. These count the number of attestations by which the attestor is ahead (if
    /// positive) or behind (if negative) for each chain.
    ///
    /// - _Attestation source chain lag_ ([`Gauge`])
    /// - _Attestation execution chain lag_ ([`Gauge`])
    ///
    /// See [`update_attestation_lag_eth`] and [`update_attestation_lag_cc3`] for implementation
    /// details.
    ///
    /// ✅ A **positive value** for the execution chain indicates that the attestor is able to
    /// keep ahead of finality.
    ///
    /// ⚠️ A **positive value** for the source chain indicates that the source chain has stalled.
    ///
    /// ⚠️ A **negative value** for the source chain indicates we are not producing attestations
    /// fast enough.
    ///
    /// ⚠️ A **negative value** for the execution chain indicates we are not receiving/validating
    /// attestations fast enough.
    ///
    /// ❌ A **large positive value** for the execution chain indicates the runtime is being
    /// overwhelmed.
    ///
    /// [`Gauge`]: prometheus_client::metrics::gauge::Gauge
    /// [`update_attestation_lag_eth`]: Self::update_attestation_lag_eth
    /// [`update_attestation_lag_cc3`]: Self::update_attestation_lag_cc3
    pub metrics_lag: prometheus_client::metrics::family::Family<
        labels::LabelAttestationLag,
        prometheus_client::metrics::gauge::Gauge,
    >,

    /// Metrics which keep track of elapsed time throughout an attestation’s lifecycle.
    ///
    /// - _Attestation production delay_ ([`Histogram`])
    /// - _Attestation quorum delay_ ([`Histogram`])
    /// - _Attestation finalization delay_ ([`Histogram`])
    ///
    /// See [`update_attestation_handler_delay`], [`update_attestation_delay_quorum`] and
    /// [`update_attestation_delay_finalization`] for implementation details.
    ///
    /// ✅ Values **converging to a low time interval** indicates all is well.
    ///
    /// ⚠️ **Outliers in attestation production** indicate we are hashing either much larger or much
    /// smaller source chain blocks.
    ///
    /// ⚠️ **Outliers in quorum delay** indicate the attestation network is either under strain, or
    /// recovering from strain.
    ///
    /// ❌ **Outliers in finalization** indicate the attestation network is stalling, or recovering
    /// from a stall.
    ///
    /// [`Histogram`]: prometheus_client::metrics::histogram::Histogram
    /// [`update_attestation_handler_delay`]: Self::update_attestation_handler_delay
    /// [`update_attestation_delay_quorum`]: Self::update_attestation_delay_quorum
    /// [`update_attestation_delay_finalization`]: Self::update_attestation_delay_finalization
    pub metrics_delay: prometheus_client::metrics::family::Family<
        labels::LabelAttestationLifecycle,
        prometheus_client::metrics::histogram::Histogram,
    >,

    /// Gauges for the attestor p2p network's health.
    ///
    /// - _Kademlia routing-table peer count_ ([`Gauge`], `peer_to_peer="routing_peers"`)
    ///
    /// This counts **entries in the Kademlia routing table**, not currently-connected peers —
    /// libp2p adds an entry on `RoutingUpdated::is_new_peer` and removes it on
    /// `RoutingUpdated::old_peer` eviction, independently of whether any connection to that
    /// peer is currently open. A routed peer with no live connection is still counted here,
    /// and the metric never decrements on `ConnectionClosed`. See [`note_routing_peer_added`]
    /// and [`note_routing_peer_evicted`].
    ///
    /// For traffic-rate dashboards see [`metrics_p2p_messages`] (Counter); for actual connected
    /// peers, see [`metrics_connected_peers`] which is incremented on `ConnectionEstablished`
    /// and decremented on `ConnectionClosed`.
    ///
    /// ✅ Routing-table size **comfortably above quorum** indicates a healthy mesh.
    /// ⚠️ Routing-table size **at quorum** is borderline; gossip mesh may be smaller.
    /// ❌ Routing-table size **below quorum** indicates a failed networking policy or that
    /// valid nodes have been taken down.
    ///
    /// [`Gauge`]: prometheus_client::metrics::gauge::Gauge
    /// [`note_routing_peer_added`]: Self::note_routing_peer_added
    /// [`note_routing_peer_evicted`]: Self::note_routing_peer_evicted
    /// [`metrics_p2p_messages`]: Self::metrics_p2p_messages
    /// [`metrics_connected_peers`]: Self::metrics_connected_peers
    pub metrics_p2p: prometheus_client::metrics::family::Family<
        labels::LabelPeerToPeer,
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,
    >,

    /// Currently connected p2p peers (distinct remote peers, not raw connections).
    ///
    /// Bumped only when libp2p `ConnectionEstablished` fires with `num_established == 1`
    /// (first established connection to that peer), and decremented only when
    /// `ConnectionClosed` fires with `num_established == 0` (last remaining connection
    /// closed). The swarm enables TCP + QUIC and allows multiple connections per peer, so
    /// counting every connection event would inflate the gauge above the actual peer count.
    ///
    /// This is distinct from [`metrics_p2p`] (Kademlia routing-table size): routing-table
    /// entries and live connections drift apart in normal operation — a peer can stay in the
    /// routing table without an active connection — so operators querying “how many attestors
    /// am I actually talking to right now” need this separate gauge.
    pub metrics_connected_peers:
        prometheus_client::metrics::gauge::Gauge<u64, std::sync::atomic::AtomicU64>,

    /// Counter for gossipsub message traffic. Monotonically increasing — meant to be consumed
    /// via the PromQL [`rate`] function to analyze message-frequency variance.
    ///
    /// Was previously a `Gauge` in `metrics_p2p`, but `inc()`-only metrics belong in a
    /// `Counter` so PromQL `rate()` / `increase()` work correctly across pod restarts (a
    /// gauge would "reset" silently and confuse rate computations).
    ///
    /// See [`increase_gossipsub_message_count`] for implementation details.
    ///
    /// ✅ A **steady rate** indicates votes are being broadcast and liveness is maintained.
    /// ⚠️ A **decreasing rate** indicates the attestation network is under strain.
    /// ⚠️ An **increasing rate** indicates the attestation network is recovering from strain.
    ///
    /// [`rate`]: https://prometheus.io/docs/prometheus/latest/querying/functions/#rate
    /// [`increase_gossipsub_message_count`]: Self::increase_gossipsub_message_count
    pub metrics_p2p_messages: prometheus_client::metrics::family::Family<
        labels::LabelPeerToPeerMessages,
        prometheus_client::metrics::counter::Counter<u64, std::sync::atomic::AtomicU64>,
    >,

    /// Metrics which keep track of failed state.
    ///
    /// - _Known invalid attestations_ ([`Counter`])
    /// - _Know equivocations_ ([`Counter`])
    /// - _Invalid gossipsub messages_ ([`Counter`])
    /// - _Failed connections_ ([`Counter`])
    ///
    /// The failed connection count is meant to be interpreted as the rate of failed p2p handshakes
    /// via the PromQL [`rate`] function.
    ///
    /// ✅ Invalid attestations, invalid message and equivocations being **zero** indicates that all
    /// is well.
    ///
    /// ✅ **Small periodic failures in connection is not an issue**. Gossipsub periodically
    /// refreshes its peers to keep its peerset up to date and protect against eclipse attacks. As
    /// part of this, the protocol might attempt to handshake with incompatible peers if they are
    /// discoverable in the local network (this is the case for example if incompatible nodes enable
    /// MDns discovery).
    ///
    /// ⚠️ **Large, repeated and continuous spikes in failed connection is an issue** as
    /// it indicates either a bug in the attestor code or of an attack.
    ///
    /// ❌ Invalid attestations, invalid messages or equivocations being **greater than zero**
    /// indicates either a critical bug in the attestor code or that we are under attack.
    ///
    /// [`Counter`]: prometheus_client::metrics::counter::Counter
    /// [`rate`]: https://prometheus.io/docs/prometheus/latest/querying/functions/#rate
    pub metrics_error: prometheus_client::metrics::family::Family<
        labels::LabelFailedState,
        prometheus_client::metrics::counter::Counter<u64, std::sync::atomic::AtomicU64>,
    >,
}

impl Metrics {
    pub fn new(config: Config) -> Self {
        let mut registry = prometheus_client::registry::Registry::default();
        let metrics_production = prometheus_client::metrics::family::Family::default();
        let metrics_lag = prometheus_client::metrics::family::Family::default();
        let metrics_hardware = prometheus_client::metrics::family::Family::default();
        let metrics_delay = prometheus_client::metrics::family::Family::<
            labels::LabelAttestationLifecycle,
            _,
        >::new_with_constructor(|| {
            prometheus_client::metrics::histogram::Histogram::new(
                prometheus_client::metrics::histogram::exponential_buckets(0.01, 2.0, 15),
            )
        });
        let metrics_p2p = prometheus_client::metrics::family::Family::default();
        let metrics_p2p_messages = prometheus_client::metrics::family::Family::default();
        let metrics_connected_peers =
            prometheus_client::metrics::gauge::Gauge::<u64, std::sync::atomic::AtomicU64>::default(
            );
        let metrics_error = prometheus_client::metrics::family::Family::default();

        registry.register(
            "attestor",
            "Basic operational information",
            prometheus_client::metrics::info::Info::new(items::MetricsInfo {
                name: config.name,
                address: config.address.to_string(),
                peer_id: config.peer_id.to_string(),
                chain_key: config.chain_key,
            }),
        );

        registry.register(
            "production",
            "Progress in attestation production and finalization",
            metrics_production.clone(),
        );

        registry.register(
            "lag",
            "Lag in attestation production, per chain",
            metrics_lag.clone(),
        );

        registry.register(
            "hardware",
            "Hardware usage metrics",
            metrics_hardware.clone(),
        );

        registry.register(
            "attestation_delay",
            "Attestation processing delay per lifecycle stage",
            metrics_delay.clone(),
        );

        registry.register(
            "peer_to_peer",
            "Peer-to-peer networking gauges (Kademlia routing-table size)",
            metrics_p2p.clone(),
        );

        registry.register(
            "peer_to_peer_messages",
            "Peer-to-peer message-traffic counters (consume via PromQL rate())",
            metrics_p2p_messages.clone(),
        );

        registry.register(
            "connected_peers",
            "Currently connected p2p peers (ConnectionEstablished minus ConnectionClosed)",
            metrics_connected_peers.clone(),
        );

        registry.register(
            "failed_states",
            "Counts of various failure states",
            metrics_error.clone(),
        );

        let metrics = Self(std::sync::Arc::new(Store {
            registry,
            metrics_production,
            metrics_lag,
            metrics_hardware,
            metrics_delay,
            metrics_p2p,
            metrics_p2p_messages,
            metrics_connected_peers,
            metrics_error,
        }));

        let attestation_latest_cc3 = config
            .start_attestation
            .map(|info| info.height)
            .unwrap_or(config.genesis);

        metrics.set_attestation_finalized(attestation_latest_cc3);
        metrics.set_attestation_local(attestation_latest_cc3);

        let attestation_local = config
            .start_height
            .saturating_sub(config.attestation_interval.get());
        metrics.update_attestation_lag_eth(
            attestation_local,
            config.attestation_latest_eth,
            config.attestation_interval,
        );
        metrics.update_attestation_lag_cc3(
            attestation_local,
            attestation_latest_cc3,
            config.attestation_interval,
        );

        metrics
    }

    pub fn encode(&self) -> String {
        let mut buffer = String::new();
        prometheus_client::encoding::text::encode(&mut buffer, &self.0.registry).unwrap();
        buffer
    }

    pub async fn update_hardware(&self) {
        if let Ok(pid) = sysinfo::get_current_pid() {
            // We initialize a new hardware interface on each call to avoid having to acquire a
            // blocking lock on a global resource due to mutable requirements on
            // `refresh_specifics`.
            let specifics = sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram())
                .with_processes(
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory(),
                );
            let mut sys = sysinfo::System::new_with_specifics(specifics);

            // NOTE: CPU USAGE
            //
            // From the sysinfo docs: "Please note that the result [of calling global_cpu_usage]
            // will very likely be inaccurate at the first call. You need to call
            // [refresh_cpu_usage] at least twice (with a bit of time between each call, like 200
            // ms, take a look at MINIMUM_CPU_UPDATE_INTERVAL for more information) to get accurate
            // value as it uses previous results to compute the next value."
            tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
            sys.refresh_specifics(specifics);

            // NOTE: METRICS
            //
            // Methods like `global_cpu_usage` target system-wide hardware metrics -we need to make
            // sure we retrieve hardware data for the current process only.
            if let Some(process) = sys.process(pid) {
                let cpu_process = process.cpu_usage() as f64;
                let cpu_count = sys.cpus().len() as f64;
                let usage_cpu = cpu_process / cpu_count;

                let memory_process = process.memory() as f64;
                let memory_total = sys.total_memory() as f64;
                let usage_memory = (memory_process / memory_total) * 100.0;

                self.0
                    .metrics_hardware
                    .get_or_create(&labels::LabelHardware {
                        hardware: labels::Hardware::Cpu,
                    })
                    .set(usage_cpu);
                self.0
                    .metrics_hardware
                    .get_or_create(&labels::LabelHardware {
                        hardware: labels::Hardware::Memory,
                    })
                    .set(usage_memory);
            }
        };
    }

    pub fn set_attestation_local(&self, height: attestor_primitives::Height) {
        self.0
            .metrics_production
            .get_or_create(&labels::LabelAttestationProgress {
                progress: labels::AttestationProgress::Local,
            })
            .set(height);
    }

    pub fn set_attestation_finalized(&self, height: attestor_primitives::Height) {
        self.0
            .metrics_production
            .get_or_create(&labels::LabelAttestationProgress {
                progress: labels::AttestationProgress::Finalized,
            })
            .set(height);
    }

    pub fn update_attestation_lag_eth(
        &self,
        attestation_local: attestor_primitives::Height,
        block_latest_eth: attestor_primitives::Height,
        interval: std::num::NonZero<attestor_primitives::Height>,
    ) {
        let attestation_local = attestation_local as i64;
        let attestation_latest_eth = block_latest_eth as i64;
        let interval = interval.get() as i64;
        let lag_eth = attestation_local.saturating_sub(attestation_latest_eth) / interval;

        self.0
            .metrics_lag
            .get_or_create(&labels::LabelAttestationLag {
                source: labels::AttestationLagSource::Eth,
            })
            .set(lag_eth);
    }

    pub fn update_attestation_lag_cc3(
        &self,
        attestation_local: attestor_primitives::Height,
        attestation_latest_cc3: attestor_primitives::Height,
        interval: std::num::NonZero<attestor_primitives::Height>,
    ) {
        use prometheus_client::metrics::gauge::Atomic as _;

        let attestation_local = attestation_local as i64;
        let attestation_latest_cc3 = attestation_latest_cc3 as i64;
        let interval = interval.get() as i64;
        let lag_cc3 = attestation_local.saturating_sub(attestation_latest_cc3) / interval;

        self.0
            .metrics_lag
            .get_or_create(&labels::LabelAttestationLag {
                source: labels::AttestationLagSource::CC3,
            })
            .inner()
            .set(lag_cc3);
    }

    /// Observe handler-delay time for the local-emit path: the duration between
    /// `StreamAttestation` yielding a finished attestation and the production task finishing
    /// its pool-insert + gossip + cache plumbing for it.
    ///
    /// This does NOT include root fetching, merkle-tree construction, or BLS signing — those
    /// happen inside `StreamAttestation` *before* it yields. The metric name preserved here
    /// (`Production`) is kept for dashboard compatibility, but the audit's complaint about it
    /// implying total-generation-latency is real; callers and dashboards should treat this as
    /// handler-delay only. See [`update_attestation_handler_delay`].
    ///
    /// [`update_attestation_handler_delay`]: Self::update_attestation_handler_delay
    pub fn update_attestation_handler_delay(&self, delay: std::time::Duration) {
        self.0
            .metrics_delay
            .get_or_create(&labels::LabelAttestationLifecycle {
                lifecycle: labels::AttestationLifecycle::Production,
            })
            .observe(delay.as_secs_f64());
    }

    /// Note a peer was added to the Kademlia routing table. NOT a connection event — see
    /// `metrics_p2p` doc for what this gauge actually measures.
    pub fn note_routing_peer_added(&self) {
        self.0
            .metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::RoutingPeers,
            })
            .inc();
    }

    /// Note a peer was evicted from the Kademlia routing table.
    pub fn note_routing_peer_evicted(&self) {
        self.0
            .metrics_p2p
            .get_or_create(&labels::LabelPeerToPeer {
                peer_to_peer: labels::PeerToPeer::RoutingPeers,
            })
            .dec();
    }

    /// Increment the *currently connected* peer gauge — call from libp2p
    /// `SwarmEvent::ConnectionEstablished` **only when `num_established == 1`** (first
    /// connection to the remote peer). Subsequent connections from the same peer (additional
    /// transports, parallel dials) must not bump this gauge or it stops representing distinct
    /// peers. Independent from [`Self::note_routing_peer_added`], which tracks the Kademlia
    /// routing-table.
    pub fn note_peer_connected(&self) {
        self.0.metrics_connected_peers.inc();
    }

    /// Decrement the *currently connected* peer gauge — call from libp2p
    /// `SwarmEvent::ConnectionClosed` **only when `num_established == 0`** (last connection
    /// to the remote peer closed). Counterpart to [`Self::note_peer_connected`].
    pub fn note_peer_disconnected(&self) {
        self.0.metrics_connected_peers.dec();
    }

    pub fn increase_gossipsub_message_count(&self) {
        self.0
            .metrics_p2p_messages
            .get_or_create(&labels::LabelPeerToPeerMessages {
                kind: labels::PeerToPeerMessages::Gossipsub,
            })
            .inc();
    }

    /// Count a USC write-ability message vote that was accepted and counted toward quorum.
    pub fn note_message_vote(&self) {
        self.0
            .metrics_p2p_messages
            .get_or_create(&labels::LabelPeerToPeerMessages {
                kind: labels::PeerToPeerMessages::MessageVote,
            })
            .inc();
    }

    pub fn increase_invalid_attestation_count(&self) {
        self.0
            .metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::InvalidAttestations,
            })
            .inc();
    }

    /// Observe time-to-quorum for one height. Same observation that
    /// `MetricsAttestationPool::update_attestation_delay_quorum` produces; exposed as an
    /// inherent pub method so consumers don't need to import the legacy pool's trait.
    pub fn observe_quorum_delay(&self, delay: std::time::Duration) {
        self.0
            .metrics_delay
            .get_or_create(&labels::LabelAttestationLifecycle {
                lifecycle: labels::AttestationLifecycle::Quorum,
            })
            .observe(delay.as_secs_f64());
    }

    pub fn increase_equivocation_count(&self) {
        self.0
            .metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::Equivocations,
            })
            .inc();
    }

    pub fn increase_invalid_gossipsub_count(&self) {
        self.0
            .metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::GossipsubMessages,
            })
            .inc();
    }

    pub fn increase_connection_failure_count(&self) {
        self.0
            .metrics_error
            .get_or_create(&labels::LabelFailedState {
                failed_state: labels::FailedState::ConnectionFailures,
            })
            .inc();
    }
}

impl Metrics {
    /// Quorum-delay histogram: time from first vote to quorum being observed for a height.
    /// Inherent pub method (used to be behind a crate-private trait in the legacy pool).
    /// Note: a thinner inherent helper `observe_quorum_delay` exists above with the same
    /// semantics — kept for backward compatibility, both record into the same Histogram.
    pub fn update_attestation_delay_quorum(&self, delay: std::time::Duration) {
        self.observe_quorum_delay(delay);
    }

    /// Finalization-delay histogram: time from first vote to on-chain finalization.
    pub fn update_attestation_delay_finalization(&self, delay: std::time::Duration) {
        self.0
            .metrics_delay
            .get_or_create(&labels::LabelAttestationLifecycle {
                lifecycle: labels::AttestationLifecycle::Finalization,
            })
            .observe(delay.as_secs_f64());
    }
}

mod items {
    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct MetricsInfo {
        pub name: String,
        pub address: String,
        pub peer_id: String,
        pub chain_key: attestor_primitives::ChainKey,
    }
}

mod labels {
    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum AttestationProgress {
        Local,
        Finalized,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelAttestationProgress {
        pub progress: AttestationProgress,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum AttestationLagSource {
        Eth,
        CC3,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelAttestationLag {
        pub source: AttestationLagSource,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum Hardware {
        Cpu,
        Memory,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelHardware {
        pub hardware: Hardware,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum AttestationLifecycle {
        Production,
        Quorum,
        Finalization,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelAttestationLifecycle {
        pub lifecycle: AttestationLifecycle,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum PeerToPeer {
        /// Entries in the Kademlia routing table — NOT live connections. Renamed from `Peers`
        /// after the audit noted that the previous label implied a connection-count semantic
        /// that the code never delivered (no `ConnectionClosed` decrement, etc.).
        RoutingPeers,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelPeerToPeer {
        pub peer_to_peer: PeerToPeer,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum PeerToPeerMessages {
        /// Total gossipsub messages received by this peer. Was a `Gauge` under `metrics_p2p`
        /// before — moved to a `Counter` family so PromQL `rate()` is well-defined.
        Gossipsub,
        /// USC write-ability message votes that were accepted and counted toward quorum.
        MessageVote,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelPeerToPeerMessages {
        pub kind: PeerToPeerMessages,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelValue)]
    pub enum FailedState {
        InvalidAttestations,
        Equivocations,
        GossipsubMessages,
        ConnectionFailures,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, prometheus_client::encoding::EncodeLabelSet)]
    pub struct LabelFailedState {
        pub failed_state: FailedState,
    }
}
