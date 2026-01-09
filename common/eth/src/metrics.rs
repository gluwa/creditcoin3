use prometheus::{Error, IntCounter, IntGauge, PrometheusRegister, Registry};

#[derive(Clone, Debug)]
pub struct BlockCacheMetrics {
    pub cache_hits: IntCounter,
    pub cache_misses: IntCounter,
    pub total_cached_blocks: IntGauge,
}

impl Default for BlockCacheMetrics {
    fn default() -> Self {
        Self {
            cache_hits: IntCounter::new(
                "number_of_cache_hits",
                "The number of cache hits in the block cache",
            )
            .unwrap(),
            cache_misses: IntCounter::new(
                "number_of_cache_misses",
                "The number of cache misses in the block cache",
            )
            .unwrap(),
            total_cached_blocks: IntGauge::new(
                "total_cached_blocks",
                "Total number of cached blocks",
            )
            .unwrap(),
        }
    }
}

impl PrometheusRegister for BlockCacheMetrics {
    const DESCRIPTION: &'static str = "block_cache";
    fn register(registry: &Registry) -> Result<Self, Error> {
        let cache_hits = IntCounter::new(
            "number_of_cache_hits",
            "The number of cache hits in the block cache",
        )?;
        registry.register(Box::new(cache_hits.clone()))?;

        let cache_misses = IntCounter::new(
            "number_of_cache_misses",
            "The number of cache misses in the block cache",
        )?;
        registry.register(Box::new(cache_misses.clone()))?;

        let total_cached_blocks =
            IntGauge::new("total_cached_blocks", "Total number of cached blocks")?;
        registry.register(Box::new(total_cached_blocks.clone()))?;

        Ok(Self {
            cache_hits,
            cache_misses,
            total_cached_blocks,
        })
    }
}

impl BlockCacheMetrics {
    pub fn observe_cache_hit(&self) {
        self.cache_hits.inc();
    }

    pub fn observe_cache_miss(&self) {
        self.cache_misses.inc();
    }

    pub fn set_total_cached_blocks(&self, value: i64) {
        self.total_cached_blocks.set(value);
    }
}
