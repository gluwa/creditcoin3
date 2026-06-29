//! Tiny in-process cache of **finalized** source blocks, keyed by block number.
//!
//! The prover only fetches finalized / attested blocks (height ≤ attested height), which are
//! immutable — they cannot reorg — so cached entries never need invalidation. Consecutive and
//! batched proof requests rebuild overlapping continuity ranges from the same checkpoint
//! boundary, so without a cache the same low-numbered blocks are re-fetched (2 RPC each) and
//! re-merkleized on every request. This cache removes that repeat work entirely — in process,
//! with no external dependency and no network hop (unlike the old Redis block cache).
//!
//! Eviction keeps the highest block numbers (drops the lowest when over capacity), matching the
//! access pattern: contiguous ranges with a monotonically advancing query height keep the most
//! recent blocks hot.
//!
//! NOTE: keyed by block number only. This is correct as long as a given client fetches a single
//! [`EncodingVersion`](usc_abi_encoding::common::EncodingVersion) (the prover is V1-only). Mixing
//! encodings on one client would require an encoding-aware key.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Mutex;

use crate::OrderedBlock;

/// Bounded, in-process, block-number-keyed cache of finalized blocks.
#[derive(Debug)]
pub struct MemBlockCache {
    capacity: usize,
    inner: Mutex<BTreeMap<u64, OrderedBlock>>,
}

impl MemBlockCache {
    /// Create a cache holding at most `capacity` blocks.
    #[must_use]
    pub fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity: capacity.get(),
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    /// Return a cached block, if present. Clones the entry so the lock is released immediately.
    pub fn get(&self, number: u64) -> Option<OrderedBlock> {
        self.lock().get(&number).cloned()
    }

    /// Insert a block, evicting the lowest-numbered entries if over capacity.
    pub fn insert(&self, number: u64, block: OrderedBlock) {
        let mut map = self.lock();
        map.insert(number, block);
        evict_to_capacity(&mut map, self.capacity);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, OrderedBlock>> {
        // A poisoned lock only happens if a holder panicked while mutating; the map is plain data
        // with no broken invariant, so recover the guard rather than propagate the panic.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Drop the lowest keys until `map` holds at most `capacity` entries. Generic over the value so
/// the eviction policy can be tested without constructing an `OrderedBlock`.
fn evict_to_capacity<V>(map: &mut BTreeMap<u64, V>, capacity: usize) {
    while map.len() > capacity {
        // `BTreeMap` iterates in ascending key order, so the first key is the lowest.
        let Some((&lowest, _)) = map.iter().next() else {
            break;
        };
        map.remove(&lowest);
    }
}

#[cfg(test)]
mod tests {
    use super::evict_to_capacity;
    use std::collections::BTreeMap;

    #[test]
    fn evicts_lowest_keys_keeping_the_highest() {
        let mut map: BTreeMap<u64, u64> = (1..=10).map(|n| (n, n)).collect();
        evict_to_capacity(&mut map, 3);
        // The 3 highest block numbers survive; the lowest 7 are evicted.
        assert_eq!(map.keys().copied().collect::<Vec<_>>(), vec![8, 9, 10]);
    }

    #[test]
    fn under_capacity_is_a_no_op() {
        let mut map: BTreeMap<u64, u64> = (1..=3).map(|n| (n, n)).collect();
        evict_to_capacity(&mut map, 10);
        assert_eq!(map.len(), 3);
    }
}
