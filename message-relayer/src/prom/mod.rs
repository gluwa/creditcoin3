//! Prometheus metrics for the message relayer.
//!
//! Layout follows `proof-gen-api-server/src/prom/mod.rs`: a [`MetricsTrait`] that the runtime
//! talks to, a [`RelayerMetrics`] struct that owns the registry, and a [`NoopMetrics`]
//! implementation for tests. The metric set covers the signals the PoC PDF §10 calls out.

use std::fmt::Debug;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prometheus_client::encoding::{EncodeLabelSet, EncodeLabelValue};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::metrics::info::Info;
use prometheus_client::registry::Registry;

/// Trait for the metrics surface the runtime depends on. Allows swapping in a no-op for tests.
pub trait MetricsTrait: Send + Sync + Debug {
    fn inc_messages_indexed(&self, chain_key: u64);
    fn inc_vote(&self, chain_key: u64, outcome: VoteOutcome);
    fn observe_votes_per_message(&self, count: u64);
    fn inc_deliver_tx(&self, chain_key: u64, status: DeliveryStatus);
    fn observe_time_to_threshold(&self, duration: Duration);
    fn observe_time_to_deliver(&self, duration: Duration);
    fn set_p2p_peer_count(&self, chain_key: u64, count: i64);
    fn set_pool_messages_pending(&self, count: i64);
}

/// Shared trait object — used to plumb metrics through services without leaking the concrete type.
pub type Metrics = Arc<dyn MetricsTrait>;

/// No-op metrics for testing or when metrics are disabled.
#[derive(Debug, Default)]
pub struct NoopMetrics;

impl NoopMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl MetricsTrait for NoopMetrics {
    fn inc_messages_indexed(&self, _chain_key: u64) {}
    fn inc_vote(&self, _chain_key: u64, _outcome: VoteOutcome) {}
    fn observe_votes_per_message(&self, _count: u64) {}
    fn inc_deliver_tx(&self, _chain_key: u64, _status: DeliveryStatus) {}
    fn observe_time_to_threshold(&self, _duration: Duration) {}
    fn observe_time_to_deliver(&self, _duration: Duration) {}
    fn set_p2p_peer_count(&self, _chain_key: u64, _count: i64) {}
    fn set_pool_messages_pending(&self, _count: i64) {}
}

/// Concrete metrics container.
#[derive(Debug)]
pub struct RelayerMetrics {
    registry: Registry,
    messages_indexed: Family<LabelChain, Counter<u64, AtomicU64>>,
    votes: Family<LabelVote, Counter<u64, AtomicU64>>,
    votes_per_message: Histogram,
    deliver_tx: Family<LabelDelivery, Counter<u64, AtomicU64>>,
    time_to_threshold_seconds: Histogram,
    time_to_deliver_seconds: Histogram,
    p2p_peer_count: Family<LabelChain, Gauge<i64, AtomicI64>>,
    pool_messages_pending: Gauge<i64, AtomicI64>,
    cpu_usage_percent: Gauge<f64, AtomicU64>,
    memory_usage_bytes: Gauge<f64, AtomicU64>,
    thread_count: Gauge<i64, AtomicI64>,
    #[allow(dead_code)]
    start_time_seconds: Gauge<f64, AtomicU64>,
}

impl RelayerMetrics {
    pub fn new(chain_keys: &[u64]) -> Self {
        let mut registry = Registry::default();

        let messages_indexed = Family::default();
        registry.register(
            "relayer_messages_indexed",
            "Finalized MessagePublished events seen",
            messages_indexed.clone(),
        );

        let votes = Family::default();
        registry.register(
            "relayer_votes_received",
            "Attestor votes received over the P2P mesh, by outcome",
            votes.clone(),
        );

        let votes_per_message = Histogram::new(exponential_buckets(1.0, 2.0, 10));
        registry.register(
            "relayer_votes_per_message",
            "Distinct signers per message at the moment of delivery",
            votes_per_message.clone(),
        );

        let deliver_tx = Family::default();
        registry.register(
            "relayer_deliver_tx",
            "Inbox.deliverMessage transaction outcomes",
            deliver_tx.clone(),
        );

        let time_to_threshold_seconds = Histogram::new(exponential_buckets(0.1, 2.0, 14));
        registry.register(
            "relayer_time_to_threshold_seconds",
            "Time from MessagePublished to threshold reached",
            time_to_threshold_seconds.clone(),
        );

        let time_to_deliver_seconds = Histogram::new(exponential_buckets(0.1, 2.0, 14));
        registry.register(
            "relayer_time_to_deliver_seconds",
            "Time from threshold reached to delivery confirmed",
            time_to_deliver_seconds.clone(),
        );

        let p2p_peer_count = Family::default();
        registry.register(
            "relayer_p2p_peer_count",
            "Number of peers in the gossipsub mesh per chain_key",
            p2p_peer_count.clone(),
        );

        let pool_messages_pending = Gauge::default();
        registry.register(
            "relayer_pool_messages_pending",
            "Messages currently held in the vote pool awaiting threshold",
            pool_messages_pending.clone(),
        );

        let cpu_usage_percent = Gauge::default();
        registry.register(
            "relayer_cpu_usage_percent",
            "Process CPU usage percentage",
            cpu_usage_percent.clone(),
        );

        let memory_usage_bytes = Gauge::default();
        registry.register(
            "relayer_memory_usage_bytes",
            "Process memory usage in bytes",
            memory_usage_bytes.clone(),
        );

        let thread_count = Gauge::default();
        registry.register(
            "relayer_thread_count",
            "Number of active threads",
            thread_count.clone(),
        );

        let start_time_seconds = Gauge::default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before Unix epoch")
            .as_secs_f64();
        start_time_seconds.set(now);
        registry.register(
            "relayer_start_time_seconds",
            "Process start time as Unix timestamp (use time() - this for uptime)",
            start_time_seconds.clone(),
        );

        registry.register(
            "relayer_server",
            "Relayer information",
            Info::new(items::ServerInfo {
                chain_keys: chain_keys
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            }),
        );

        Self {
            registry,
            messages_indexed,
            votes,
            votes_per_message,
            deliver_tx,
            time_to_threshold_seconds,
            time_to_deliver_seconds,
            p2p_peer_count,
            pool_messages_pending,
            cpu_usage_percent,
            memory_usage_bytes,
            thread_count,
            start_time_seconds,
        }
    }

    pub fn encode(&self) -> String {
        let mut buffer = String::new();
        prometheus_client::encoding::text::encode(&mut buffer, &self.registry).unwrap();
        buffer
    }

    pub fn build_metrics_response(&self) -> axum::response::Response {
        axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )
            .body(axum::body::Body::from(self.encode()))
            .unwrap()
    }

    /// Spawn a background task that periodically updates hardware gauges. Mirrors the helper
    /// in `proof-gen-api-server` — the relayer's runtime calls this once at startup.
    pub fn spawn_hardware_updater(metrics: Arc<Self>) {
        tokio::spawn(async move {
            let specifics = sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram())
                .with_processes(
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory(),
                );
            let mut sys = sysinfo::System::new_with_specifics(specifics);

            tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
            sys.refresh_specifics(specifics);

            let interval = std::time::Duration::from_secs(5);
            loop {
                metrics.update_gauges_from_system(&sys);
                tokio::time::sleep(interval).await;
                sys.refresh_specifics(specifics);
            }
        });
    }

    fn update_gauges_from_system(&self, sys: &sysinfo::System) {
        if let Ok(pid) = sysinfo::get_current_pid() {
            if let Some(process) = sys.process(pid) {
                let cpu_process = f64::from(process.cpu_usage());
                let cpu_count = sys.cpus().len() as f64;
                let cpu_percent = if cpu_count > 0.0 {
                    cpu_process / cpu_count
                } else {
                    0.0
                };
                self.cpu_usage_percent.set(cpu_percent);
                self.memory_usage_bytes.set(process.memory() as f64);
                if let Some(tasks) = process.tasks() {
                    self.thread_count.set(tasks.len() as i64);
                }
            }
        }
    }
}

impl MetricsTrait for RelayerMetrics {
    fn inc_messages_indexed(&self, chain_key: u64) {
        self.messages_indexed
            .get_or_create(&LabelChain { chain_key })
            .inc();
    }

    fn inc_vote(&self, chain_key: u64, outcome: VoteOutcome) {
        self.votes
            .get_or_create(&LabelVote { chain_key, outcome })
            .inc();
    }

    fn observe_votes_per_message(&self, count: u64) {
        self.votes_per_message.observe(count as f64);
    }

    fn inc_deliver_tx(&self, chain_key: u64, status: DeliveryStatus) {
        self.deliver_tx
            .get_or_create(&LabelDelivery { chain_key, status })
            .inc();
    }

    fn observe_time_to_threshold(&self, duration: Duration) {
        self.time_to_threshold_seconds
            .observe(duration.as_secs_f64());
    }

    fn observe_time_to_deliver(&self, duration: Duration) {
        self.time_to_deliver_seconds.observe(duration.as_secs_f64());
    }

    fn set_p2p_peer_count(&self, chain_key: u64, count: i64) {
        self.p2p_peer_count
            .get_or_create(&LabelChain { chain_key })
            .set(count);
    }

    fn set_pool_messages_pending(&self, count: i64) {
        self.pool_messages_pending.set(count);
    }
}

/// Build the minimal HTTP surface (`/metrics` + `/health`).
pub fn build_router(metrics: Arc<RelayerMetrics>) -> axum::Router {
    use axum::routing::get;
    use axum::Extension;

    axum::Router::new()
        .route("/health", get(|| async { axum::http::StatusCode::OK }))
        .route(
            "/metrics",
            get(|Extension(m): Extension<Arc<RelayerMetrics>>| async move {
                m.build_metrics_response()
            }),
        )
        .layer(Extension(metrics))
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
pub enum VoteOutcome {
    Accept,
    Reject,
    Ignore,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
pub enum DeliveryStatus {
    Submitted,
    Succeeded,
    Reverted,
    AlreadyValidated,
    Pending,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct LabelChain {
    pub chain_key: u64,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct LabelVote {
    pub chain_key: u64,
    pub outcome: VoteOutcome,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct LabelDelivery {
    pub chain_key: u64,
    pub status: DeliveryStatus,
}

mod items {
    use prometheus_client::encoding::EncodeLabelSet;

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct ServerInfo {
        pub chain_keys: String,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_encode_round_trips() {
        let m = RelayerMetrics::new(&[2, 7]);
        m.inc_messages_indexed(2);
        m.inc_vote(2, VoteOutcome::Accept);
        m.inc_deliver_tx(7, DeliveryStatus::Submitted);
        let body = m.encode();
        assert!(body.contains("relayer_messages_indexed"));
        assert!(body.contains("relayer_votes_received"));
        assert!(body.contains("relayer_deliver_tx"));
        assert!(body.contains("chain_keys=\"2,7\""));
    }

    #[test]
    fn noop_metrics_compile() {
        let m = NoopMetrics::new();
        m.inc_messages_indexed(1);
        m.inc_vote(1, VoteOutcome::Reject);
        m.inc_deliver_tx(1, DeliveryStatus::Succeeded);
        m.observe_votes_per_message(7);
        m.observe_time_to_threshold(Duration::from_millis(100));
        m.observe_time_to_deliver(Duration::from_millis(200));
        m.set_p2p_peer_count(1, 4);
        m.set_pool_messages_pending(3);
    }
}
