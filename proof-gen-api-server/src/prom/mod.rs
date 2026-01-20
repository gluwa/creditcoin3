//! Prometheus metrics for proof-gen-api-server.
//!
//! This module provides comprehensive metrics following the same pattern as the attestor,
//! using the `prometheus_client` crate with type-safe labels.

use std::sync::atomic::{AtomicI64, AtomicU64};
use std::sync::Arc;
use std::thread::available_parallelism;

use eth::metrics::BlockCacheMetrics;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::metrics::info::Info;
use prometheus_client::registry::Registry;

pub use labels::*;

/// Metrics type alias for use throughout the codebase.
pub type Metrics = Arc<ProofGenMetrics>;

/// Optional metrics for when prometheus is disabled.
pub type OptionalMetrics = Option<Metrics>;

/// Comprehensive metrics for the proof-gen-api-server.
#[derive(Debug)]
pub struct ProofGenMetrics {
    registry: Registry,

    // Request metrics
    pub requests: Family<LabelRequest, Counter<u64, AtomicU64>>,
    pub request_duration: Family<LabelEndpoint, Histogram>,
    pub request_size_bytes: Family<LabelEndpoint, Histogram>,
    pub response_size_bytes: Family<LabelEndpoint, Histogram>,

    // Error metrics
    pub errors: Family<LabelError, Counter<u64, AtomicU64>>,

    // Cache metrics (for continuity proof cache)
    pub cache: Family<LabelCacheResult, Counter<u64, AtomicU64>>,

    // Proof generation metrics
    pub proof_generation_duration: Family<LabelProofType, Histogram>,
    pub proof_generation: Family<LabelProofResult, Counter<u64, AtomicU64>>,
    pub proof_blocks: Histogram,
    pub merkle_generation_duration: Histogram,
    pub proofs_stored: Gauge<i64, AtomicI64>,
    pub last_proof_generation_timestamp: Gauge<f64, AtomicU64>,
    pub proof_age_seconds: Histogram,

    // Business metrics
    pub block_range: Histogram,
    pub uptime_seconds: Gauge<f64, AtomicU64>,

    // Hardware metrics
    pub hardware: Family<LabelHardware, Gauge<f64, AtomicU64>>,
    pub thread_count: Gauge<i64, AtomicI64>,

    // Block cache metrics (for Redis block cache)
    pub block_cache_metrics: BlockCacheMetrics,
}

impl ProofGenMetrics {
    /// Create a new metrics instance and register all metrics.
    pub fn new(chain_key: u64) -> Self {
        let mut registry = Registry::default();

        // Request metrics
        let requests = Family::default();
        registry.register("proof_gen_requests", "Total API requests", requests.clone());

        let request_duration = Family::<LabelEndpoint, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.001, 2.0, 15)) // 1ms to ~30s
        });
        registry.register(
            "proof_gen_request_duration_seconds",
            "Request latency in seconds",
            request_duration.clone(),
        );

        let request_size_bytes = Family::<LabelEndpoint, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(64.0, 2.0, 16)) // 64B to ~4MB
        });
        registry.register(
            "proof_gen_request_size_bytes",
            "Request payload size in bytes",
            request_size_bytes.clone(),
        );

        let response_size_bytes = Family::<LabelEndpoint, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(64.0, 2.0, 20)) // 64B to ~64MB
        });
        registry.register(
            "proof_gen_response_size_bytes",
            "Response payload size in bytes",
            response_size_bytes.clone(),
        );

        // Error metrics
        let errors = Family::default();
        registry.register("proof_gen_errors", "Errors by type", errors.clone());

        // Cache metrics
        let cache = Family::default();
        registry.register("proof_gen_cache", "Cache operations", cache.clone());

        // Proof generation metrics
        let proof_generation_duration = Family::<LabelProofType, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.1, 2.0, 10)) // 100ms to ~100s
        });
        registry.register(
            "proof_gen_generation_duration_seconds",
            "Time to generate proofs",
            proof_generation_duration.clone(),
        );

        let proof_generation = Family::default();
        registry.register(
            "proof_gen_generation",
            "Proof generation attempts",
            proof_generation.clone(),
        );

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

        let proofs_stored = Gauge::default();
        registry.register(
            "proof_gen_proofs_stored",
            "Total number of proofs stored in the database",
            proofs_stored.clone(),
        );

        let last_proof_generation_timestamp = Gauge::default();
        registry.register(
            "proof_gen_last_proof_generation_timestamp_seconds",
            "Unix timestamp of the last successful proof generation",
            last_proof_generation_timestamp.clone(),
        );

        let proof_age_seconds = Histogram::new(exponential_buckets(0.1, 2.0, 20)); // 100ms to ~27 hours
        registry.register(
            "proof_gen_proof_age_seconds",
            "Age of proofs when served from cache",
            proof_age_seconds.clone(),
        );

        // Business metrics
        let block_range = Histogram::new(exponential_buckets(1000.0, 10.0, 8)); // 1K to 100M
        registry.register(
            "proof_gen_block_range",
            "Distribution of requested block numbers",
            block_range.clone(),
        );

        let uptime_seconds = Gauge::default();
        registry.register(
            "proof_gen_uptime_seconds",
            "Server uptime in seconds",
            uptime_seconds.clone(),
        );

        // Hardware metrics
        let hardware = Family::default();
        registry.register(
            "proof_gen_hardware",
            "Hardware utilization metrics",
            hardware.clone(),
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
            request_size_bytes,
            response_size_bytes,
            errors,
            cache,
            proof_generation_duration,
            proof_generation,
            proof_blocks,
            merkle_generation_duration,
            proofs_stored,
            last_proof_generation_timestamp,
            proof_age_seconds,
            block_range,
            uptime_seconds,
            hardware,
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
                let cpu_process = process.cpu_usage() as f64;
                let cpu_count = sys.cpus().len() as f64;
                let usage_cpu = cpu_process / cpu_count;

                let memory_process = process.memory() as f64;
                let memory_total = sys.total_memory() as f64;
                let usage_memory = (memory_process / memory_total) * 100.0;

                self.hardware
                    .get_or_create(&LabelHardware {
                        hardware: Hardware::Cpu,
                    })
                    .set(usage_cpu);
                self.hardware
                    .get_or_create(&LabelHardware {
                        hardware: Hardware::Memory,
                    })
                    .set(usage_memory);

                // Update thread count
                // process.tasks() returns actual thread count on Linux, None on other platforms
                let thread_count = process
                    .tasks()
                    .map(|tasks| tasks.len() as i64)
                    .unwrap_or_else(|| {
                        available_parallelism().map(|n| n.get() as i64).unwrap_or(1)
                    });
                self.thread_count.set(thread_count);
            }
        }
    }

    // === Request Metrics ===

    pub fn inc_request(&self, endpoint: Endpoint, status: Status) {
        self.requests
            .get_or_create(&LabelRequest { endpoint, status })
            .inc();
    }

    pub fn observe_request_duration(&self, endpoint: Endpoint, duration_secs: f64) {
        self.request_duration
            .get_or_create(&LabelEndpoint { endpoint })
            .observe(duration_secs);
    }

    pub fn observe_request_size(&self, endpoint: Endpoint, bytes: u64) {
        self.request_size_bytes
            .get_or_create(&LabelEndpoint { endpoint })
            .observe(bytes as f64);
    }

    pub fn observe_response_size(&self, endpoint: Endpoint, bytes: u64) {
        self.response_size_bytes
            .get_or_create(&LabelEndpoint { endpoint })
            .observe(bytes as f64);
    }

    // === Error Metrics ===

    pub fn inc_error(&self, error_type: ErrorType) {
        self.errors.get_or_create(&LabelError { error_type }).inc();
    }

    // === Cache Metrics ===

    pub fn inc_cache_hit(&self) {
        self.cache
            .get_or_create(&LabelCacheResult {
                result: CacheResult::Hit,
            })
            .inc();
    }

    pub fn inc_cache_miss(&self) {
        self.cache
            .get_or_create(&LabelCacheResult {
                result: CacheResult::Miss,
            })
            .inc();
    }

    pub fn inc_cache_invalidation(&self) {
        self.cache
            .get_or_create(&LabelCacheResult {
                result: CacheResult::Invalidation,
            })
            .inc();
    }

    // === Proof Generation Metrics ===

    pub fn observe_proof_generation(
        &self,
        proof_type: ProofType,
        duration_secs: f64,
        success: bool,
    ) {
        self.proof_generation_duration
            .get_or_create(&LabelProofType {
                proof_type: proof_type.clone(),
            })
            .observe(duration_secs);
        let result = if success {
            OpResult::Success
        } else {
            OpResult::Failure
        };
        self.proof_generation
            .get_or_create(&LabelProofResult { proof_type, result })
            .inc();
    }

    pub fn observe_proof_blocks(&self, count: u64) {
        self.proof_blocks.observe(count as f64);
    }

    pub fn observe_merkle_generation(&self, duration_secs: f64) {
        self.merkle_generation_duration.observe(duration_secs);
    }

    // === Proof Storage Metrics ===

    pub fn set_proofs_stored(&self, count: i64) {
        self.proofs_stored.set(count);
    }

    pub fn set_last_proof_generation_timestamp(&self, timestamp_secs: f64) {
        self.last_proof_generation_timestamp.set(timestamp_secs);
    }

    pub fn observe_proof_age(&self, age_secs: f64) {
        self.proof_age_seconds.observe(age_secs);
    }

    // === Business Metrics ===

    pub fn observe_block_range(&self, block: u64) {
        self.block_range.observe(block as f64);
    }

    pub fn set_uptime(&self, secs: f64) {
        self.uptime_seconds.set(secs);
    }
}

/// Extension trait for optional metrics.
/// Allows calling metric methods on `Option<Metrics>` without unwrapping.
#[allow(dead_code)]
pub trait MetricsExt {
    fn inc_cache_hit(&self);
    fn inc_cache_miss(&self);
    fn inc_cache_invalidation(&self);
    fn inc_error(&self, error_type: ErrorType);
    fn observe_proof_generation(&self, proof_type: ProofType, duration_secs: f64, success: bool);
    fn observe_proof_blocks(&self, count: u64);
    fn observe_request_size(&self, endpoint: Endpoint, bytes: u64);
    fn observe_response_size(&self, endpoint: Endpoint, bytes: u64);
    fn observe_merkle_generation(&self, duration_secs: f64);
    fn set_proofs_stored(&self, count: i64);
    fn set_last_proof_generation_timestamp(&self, timestamp_secs: f64);
    fn observe_proof_age(&self, age_secs: f64);
}

impl MetricsExt for OptionalMetrics {
    fn inc_cache_hit(&self) {
        if let Some(m) = self {
            m.inc_cache_hit();
        }
    }

    fn inc_cache_miss(&self) {
        if let Some(m) = self {
            m.inc_cache_miss();
        }
    }

    fn inc_cache_invalidation(&self) {
        if let Some(m) = self {
            m.inc_cache_invalidation();
        }
    }

    fn inc_error(&self, error_type: ErrorType) {
        if let Some(m) = self {
            m.inc_error(error_type);
        }
    }

    fn observe_proof_generation(&self, proof_type: ProofType, duration_secs: f64, success: bool) {
        if let Some(m) = self {
            m.observe_proof_generation(proof_type, duration_secs, success);
        }
    }

    fn observe_proof_blocks(&self, count: u64) {
        if let Some(m) = self {
            m.observe_proof_blocks(count);
        }
    }

    fn observe_request_size(&self, endpoint: Endpoint, bytes: u64) {
        if let Some(m) = self {
            m.observe_request_size(endpoint, bytes);
        }
    }

    fn observe_response_size(&self, endpoint: Endpoint, bytes: u64) {
        if let Some(m) = self {
            m.observe_response_size(endpoint, bytes);
        }
    }

    fn observe_merkle_generation(&self, duration_secs: f64) {
        if let Some(m) = self {
            m.observe_merkle_generation(duration_secs);
        }
    }

    fn set_proofs_stored(&self, count: i64) {
        if let Some(m) = self {
            m.set_proofs_stored(count);
        }
    }

    fn set_last_proof_generation_timestamp(&self, timestamp_secs: f64) {
        if let Some(m) = self {
            m.set_last_proof_generation_timestamp(timestamp_secs);
        }
    }

    fn observe_proof_age(&self, age_secs: f64) {
        if let Some(m) = self {
            m.observe_proof_age(age_secs);
        }
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
        DatabaseError,
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

    // Cache result labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum CacheResult {
        Hit,
        Miss,
        Invalidation,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelCacheResult {
        pub result: CacheResult,
    }

    // Proof type labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum ProofType {
        ContinuityOnly,
        ContinuityWithMerkle,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelProofType {
        pub proof_type: ProofType,
    }

    // Operation result labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum OpResult {
        Success,
        Failure,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelProofResult {
        pub proof_type: ProofType,
        pub result: OpResult,
    }

    // Hardware labels
    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
    pub enum Hardware {
        Cpu,
        Memory,
    }

    #[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
    pub struct LabelHardware {
        pub hardware: Hardware,
    }
}
