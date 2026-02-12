//! Block cache metrics using prometheus-client.
//!
//! Follows the same pattern as the attestor metrics implementation.

use std::sync::atomic::AtomicU64;
use std::time::Duration;

use prometheus_client::encoding::{EncodeLabelSet, EncodeLabelValue};
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;

/// Block cache metrics.
#[derive(Clone, Debug)]
pub struct BlockCacheMetrics {
    pub cache_operations: Family<LabelCacheResult, Counter<u64, AtomicU64>>,
    pub total_cached_blocks: Gauge<i64, std::sync::atomic::AtomicI64>,
    pub redis_operation_duration: Family<LabelRedisOp, Histogram>,
    pub redis_errors: Family<LabelRedisOp, Counter<u64, AtomicU64>>,
}

impl BlockCacheMetrics {
    /// Create and register block cache metrics.
    pub fn new(registry: &mut Registry) -> Self {
        let cache_operations = Family::default();
        registry.register(
            "eth_block_cache_operations",
            "Block cache operations (hits/misses)",
            cache_operations.clone(),
        );

        let total_cached_blocks = Gauge::default();
        registry.register(
            "eth_block_cache_total_cached_blocks",
            "Total source chain blocks cached in Redis (keys in DB; when Redis is dedicated to block cache)",
            total_cached_blocks.clone(),
        );

        let redis_operation_duration = Family::<LabelRedisOp, _>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.0001, 2.0, 16)) // 100us to ~6.5s
        });
        registry.register(
            "eth_block_cache_redis_operation_duration_seconds",
            "Redis operation latency in seconds",
            redis_operation_duration.clone(),
        );

        let redis_errors = Family::default();
        registry.register(
            "eth_block_cache_redis_errors",
            "Redis operation errors by operation type",
            redis_errors.clone(),
        );

        Self {
            cache_operations,
            total_cached_blocks,
            redis_operation_duration,
            redis_errors,
        }
    }

    pub fn observe_cache_hit(&self) {
        self.cache_operations
            .get_or_create(&LabelCacheResult {
                result: CacheResult::Hit,
            })
            .inc();
    }

    pub fn observe_cache_miss(&self) {
        self.cache_operations
            .get_or_create(&LabelCacheResult {
                result: CacheResult::Miss,
            })
            .inc();
    }

    pub fn set_total_cached_blocks(&self, value: i64) {
        self.total_cached_blocks.set(value);
    }

    pub fn observe_redis_operation(&self, op: RedisOp, duration: Duration) {
        self.redis_operation_duration
            .get_or_create(&LabelRedisOp { operation: op })
            .observe(duration.as_secs_f64());
    }

    pub fn inc_redis_error(&self, op: RedisOp) {
        self.redis_errors
            .get_or_create(&LabelRedisOp { operation: op })
            .inc();
    }
}

/// Cache operation result.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
pub enum CacheResult {
    Hit,
    Miss,
}

/// Label set for cache operations.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct LabelCacheResult {
    pub result: CacheResult,
}

/// Redis operation type.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelValue)]
pub enum RedisOp {
    Get,
    Set,
    Delete,
    Lock,
}

/// Label set for Redis operations.
#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct LabelRedisOp {
    pub operation: RedisOp,
}
