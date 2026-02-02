//! Prometheus metrics for proof-gen-api-server.
//!
//! This module provides comprehensive metrics following the same pattern as the attestor,
//! using the `prometheus_client` crate with type-safe labels.

use std::fmt::Debug;
use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eth::metrics::BlockCacheMetrics;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::metrics::info::Info;
use prometheus_client::registry::Registry;

// Only export the types that are actually used externally
pub use labels::{Endpoint, ErrorType, Status};

/// Trait for types that can provide their error classification.
/// Implementing this trait allows errors to self-categorize for metrics,
/// providing compile-time safety for error type mapping.
pub trait GetErrorType {
    fn error_type(&self) -> ErrorType;
}

/// Trait defining the metrics interface.
/// Implemented by both `ProofGenMetrics` (real metrics) and `NoopMetrics` (no-op for testing).
pub trait MetricsTrait: Send + Sync + Debug {
    // Request metrics
    fn inc_request(&self, endpoint: Endpoint, status: Status);
    fn observe_request_duration(&self, endpoint: Endpoint, duration: Duration);
    fn observe_request_size(&self, endpoint: Endpoint, bytes: u64);
    fn observe_response_size(&self, endpoint: Endpoint, bytes: u64);

    // Error metrics
    fn inc_error(&self, error_type: ErrorType);

    // Proof generation metrics
    fn observe_proof_blocks(&self, count: u64);
    fn observe_merkle_generation(&self, duration: Duration);
    fn set_last_proof_generation_timestamp(&self, timestamp_secs: f64);

    // Business metrics
    fn observe_block_range(&self, block: u64);
}

/// Metrics type alias for use throughout the codebase.
/// Uses trait object to support both real metrics and no-op metrics.
pub type Metrics = Arc<dyn MetricsTrait>;

/// No-op metrics implementation for testing or when metrics are disabled.
#[derive(Debug)]
pub struct NoopMetrics;

impl NoopMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl MetricsTrait for NoopMetrics {
    // Request metrics
    fn inc_request(&self, _endpoint: Endpoint, _status: Status) {}
    fn observe_request_duration(&self, _endpoint: Endpoint, _duration: Duration) {}
    fn observe_request_size(&self, _endpoint: Endpoint, _bytes: u64) {}
    fn observe_response_size(&self, _endpoint: Endpoint, _bytes: u64) {}

    // Error metrics
    fn inc_error(&self, _error_type: ErrorType) {}

    // Proof generation metrics
    fn observe_proof_blocks(&self, _count: u64) {}
    fn observe_merkle_generation(&self, _duration: Duration) {}
    fn set_last_proof_generation_timestamp(&self, _timestamp_secs: f64) {}

    // Business metrics
    fn observe_block_range(&self, _block: u64) {}
}

/// Comprehensive metrics for the proof-gen-api-server.
#[derive(Debug)]
pub struct ProofGenMetrics {
    registry: Registry,

    // Request metrics
    requests: Family<labels::LabelRequest, Counter<u64, AtomicU64>>,
    request_duration: Family<labels::LabelEndpoint, Histogram>,
    /// Transfer size in bytes (request/response differentiated by direction label)
    transfer_size_bytes: Family<labels::LabelTransfer, Histogram>,

    // Error metrics
    errors: Family<labels::LabelError, Counter<u64, AtomicU64>>,

    // Proof generation metrics
    proof_blocks: Histogram,
    merkle_generation_duration: Histogram,
    last_proof_generation_timestamp: Gauge<f64, AtomicU64>,

    // Business metrics
    block_range: Histogram,
    /// Server start time as Unix timestamp (seconds since epoch).
    /// Use PromQL `time() - proof_gen_start_time_seconds` to calculate uptime.
    /// This field is registered with Prometheus registry and accessed during encoding,
    /// so it's not directly read in code but is used by Prometheus.
    #[allow(dead_code)]
    start_time_seconds: Gauge<f64, AtomicU64>,

    // Hardware metrics
    cpu_usage_percent: Gauge<f64, AtomicU64>,
    memory_usage_bytes: Gauge<f64, AtomicU64>,
    thread_count: Gauge<i64, AtomicI64>,

    // Block cache metrics (for Redis block cache)
    block_cache_metrics: BlockCacheMetrics,
}

impl ProofGenMetrics {
    /// Create a new metrics instance and register all metrics.
    pub fn new(chain_key: u64) -> Self {
        let mut registry = Registry::default();

        // Request metrics
        let requests = Family::default();
        registry.register("proof_gen_requests", "Total API requests", requests.clone());

        let request_duration = Family::<labels::LabelEndpoint, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.001, 2.0, 15)) // 1ms to ~30s
        });
        registry.register(
            "proof_gen_request_duration_seconds",
            "Request latency in seconds",
            request_duration.clone(),
        );

        let transfer_size_bytes = Family::<labels::LabelTransfer, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(64.0, 2.0, 20)) // 64B to ~64MB
        });
        registry.register(
            "proof_gen_transfer_size_bytes",
            "Transfer payload size in bytes (by endpoint and direction)",
            transfer_size_bytes.clone(),
        );

        // Error metrics
        let errors = Family::default();
        registry.register("proof_gen_errors", "Errors by type", errors.clone());

        // Proof generation metrics
        let proof_blocks = Histogram::new(exponential_buckets(1.0, 2.0, 15)); // 1 to ~32K blocks
        registry.register(
            "proof_gen_proof_blocks",
            "Number of blocks in continuity proofs",
            proof_blocks.clone(),
        );

        let merkle_generation_duration = Histogram::new(exponential_buckets(0.001, 2.0, 12)); // 1ms to ~4s
        registry.register(
            "proof_gen_merkle_generation_duration_seconds",
            "Time to generate merkle proofs",
            merkle_generation_duration.clone(),
        );

        let last_proof_generation_timestamp = Gauge::default();
        registry.register(
            "proof_gen_last_proof_generation_timestamp_seconds",
            "Unix timestamp of the last successful proof generation",
            last_proof_generation_timestamp.clone(),
        );

        // Business metrics
        let block_range = Histogram::new(exponential_buckets(1000.0, 10.0, 8)); // 1K to 100M
        registry.register(
            "proof_gen_block_range",
            "Distribution of requested block numbers",
            block_range.clone(),
        );

        let start_time_seconds = Gauge::default();
        // Set start time once at initialization (Unix timestamp)
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("System time before Unix epoch")
            .as_secs_f64();
        start_time_seconds.set(now);
        registry.register(
            "proof_gen_start_time_seconds",
            "Server start time as Unix timestamp (use time() - this for uptime)",
            start_time_seconds.clone(),
        );

        // Hardware metrics
        let cpu_usage_percent = Gauge::default();
        registry.register(
            "proof_gen_cpu_usage_percent",
            "Process CPU usage percentage",
            cpu_usage_percent.clone(),
        );

        let memory_usage_bytes = Gauge::default();
        registry.register(
            "proof_gen_memory_usage_bytes",
            "Process memory usage in bytes",
            memory_usage_bytes.clone(),
        );

        let thread_count = Gauge::default();
        registry.register(
            "proof_gen_thread_count",
            "Number of active threads",
            thread_count.clone(),
        );

        // Server info metric
        registry.register(
            "proof_gen_server",
            "Server information",
            Info::new(items::ServerInfo { chain_key }),
        );

        // Block cache metrics (for Redis block cache, registered in the same registry)
        let block_cache_metrics = BlockCacheMetrics::new(&mut registry);

        Self {
            registry,
            requests,
            request_duration,
            transfer_size_bytes,
            errors,
            proof_blocks,
            merkle_generation_duration,
            last_proof_generation_timestamp,
            block_range,
            start_time_seconds,
            cpu_usage_percent,
            memory_usage_bytes,
            thread_count,
            block_cache_metrics,
        }
    }

    /// Get block cache metrics for use with the eth client's Redis cache.
    pub fn block_cache_metrics(&self) -> BlockCacheMetrics {
        self.block_cache_metrics.clone()
    }

    /// Encode all metrics to OpenMetrics text format.
    pub fn encode(&self) -> String {
        let mut buffer = String::new();
        prometheus_client::encoding::text::encode(&mut buffer, &self.registry).unwrap();
        buffer
    }

    /// Build metrics HTTP response with updated hardware metrics.
    /// This is a shared helper used by both the main API server and the separate metrics server.
    pub async fn build_metrics_response(&self) -> axum::response::Response {
        // Update hardware metrics before encoding
        self.update_hardware().await;

        axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(
                axum::http::header::CONTENT_TYPE,
                "application/openmetrics-text; version=1.0.0; charset=utf-8",
            )
            .body(axum::body::Body::from(self.encode()))
            .unwrap()
    }

    /// Update hardware metrics (CPU, memory usage, and thread count).
    /// Should be called before encoding metrics for fresh values.
    pub async fn update_hardware(&self) {
        if let Ok(pid) = sysinfo::get_current_pid() {
            let specifics = sysinfo::RefreshKind::nothing()
                .with_cpu(sysinfo::CpuRefreshKind::nothing().with_cpu_usage())
                .with_memory(sysinfo::MemoryRefreshKind::nothing().with_ram())
                .with_processes(
                    sysinfo::ProcessRefreshKind::nothing()
                        .with_cpu()
                        .with_memory(),
                );
            let mut sys = sysinfo::System::new_with_specifics(specifics);

            // CPU usage requires a delay between samples for accurate reading
            tokio::time::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL).await;
            sys.refresh_specifics(specifics);

            if let Some(process) = sys.process(pid) {
                // CPU usage as percentage (normalized by CPU count)
                let cpu_process = process.cpu_usage() as f64;
                let cpu_count = sys.cpus().len() as f64;
                // Avoid division by zero if CPU list is empty (can occur in containerized environments)
                let cpu_percent = if cpu_count > 0.0 {
                    cpu_process / cpu_count
                } else {
                    0.0
                };
                self.cpu_usage_percent.set(cpu_percent);

                // Memory usage in bytes
                let memory_bytes = process.memory() as f64;
                self.memory_usage_bytes.set(memory_bytes);

                // Update thread count (only available on Linux)
                if let Some(tasks) = process.tasks() {
                    self.thread_count.set(tasks.len() as i64);
                }
                // On non-Linux platforms, thread_count metric is not updated
            }
        }
        // Note: start_time_seconds is set once at init, no periodic update needed
    }
}

/// Shared handler function for metrics endpoint.
/// Can be used with either Extension or State extractor by passing the metrics.
pub async fn handle_metrics_response(
    metrics: Arc<ProofGenMetrics>,
) -> impl axum::response::IntoResponse {
    metrics.build_metrics_response().await
}

impl MetricsTrait for ProofGenMetrics {
    // Request metrics
    fn inc_request(&self, endpoint: Endpoint, status: Status) {
        self.requests
            .get_or_create(&labels::LabelRequest { endpoint, status })
            .inc();
    }

    fn observe_request_duration(&self, endpoint: Endpoint, duration: Duration) {
        self.request_duration
            .get_or_create(&labels::LabelEndpoint { endpoint })
            .observe(duration.as_secs_f64());
    }

    fn observe_request_size(&self, endpoint: Endpoint, bytes: u64) {
        self.transfer_size_bytes
            .get_or_create(&labels::LabelTransfer {
                endpoint,
                direction: labels::Direction::Request,
            })
            .observe(bytes as f64);
    }

    fn observe_response_size(&self, endpoint: Endpoint, bytes: u64) {
        self.transfer_size_bytes
            .get_or_create(&labels::LabelTransfer {
                endpoint,
                direction: labels::Direction::Response,
            })
            .observe(bytes as f64);
    }

    // Error metrics
    fn inc_error(&self, error_type: ErrorType) {
        self.errors
            .get_or_create(&labels::LabelError { error_type })
            .inc();
    }

    // Proof generation metrics
    fn observe_proof_blocks(&self, count: u64) {
        self.proof_blocks.observe(count as f64);
    }

    fn observe_merkle_generation(&self, duration: Duration) {
        self.merkle_generation_duration
            .observe(duration.as_secs_f64());
    }

    fn set_last_proof_generation_timestamp(&self, timestamp_secs: f64) {
        self.last_proof_generation_timestamp.set(timestamp_secs);
    }

    // Business metrics
    fn observe_block_range(&self, block: u64) {
        self.block_range.observe(block as f64);
    }
}

/// Info metric items following the attestor pattern.
mod items {
    use prometheus_client::encoding::EncodeLabelSet;

    /// Server info for the info metric.
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct ServerInfo {
        pub chain_key: u64,
    }
}

/// Label definitions following the attestor pattern.
mod labels {
    use prometheus_client::encoding::{EncodeLabelSet, EncodeLabelValue};

    // Endpoint labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum Endpoint {
        Proof,
        ProofWithTx,
        ProofByTxHash,
        Health,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelEndpoint {
        pub endpoint: Endpoint,
    }

    // Transfer direction labels (for request/response size)
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum Direction {
        Request,
        Response,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelTransfer {
        pub endpoint: Endpoint,
        pub direction: Direction,
    }

    // Request status labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum Status {
        Success,
        ClientError,
        ServerError,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelRequest {
        pub endpoint: Endpoint,
        pub status: Status,
    }

    // Error type labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum ErrorType {
        BlockNotReady,
        BlockBeforeGenesis,
        BlockNotOnSourceChain,
        RpcUnavailable,
        MerkleError,
        InvalidParameter,
        TxHashNotFound,
        TxIndexOutOfBounds,
        AttestationsMissing,
        QueryOutOfRange,
        TxHashLookupUnavailable,
        Internal,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelError {
        pub error_type: ErrorType,
    }
}
