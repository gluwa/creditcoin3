//! Sled-backed root store.
//!
//! Schema: key = block height (u64 big-endian, 8 bytes), value = merkle root (H256, 32 bytes).
//! Big-endian keys ensure sled's sorted iteration yields blocks in height order.
//!
//! A separate `meta` tree holds a persisted entry counter so we avoid the O(n)
//! `db.len()` startup scan that warms sled's page cache with the entire history.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use sp_core::H256;

/// Soft cap for sled's page cache. The 0.34 default is ~1 GiB which is wildly
/// oversized for a 40-byte-per-record workload; cap it so resident memory
/// stays predictable. This is a hint, not a hard limit — sled may still
/// exceed it under bursty write load.
const SLED_CACHE_CAPACITY_BYTES: u64 = 64 * 1024 * 1024;

/// Name of the side tree used for archiver-internal metadata
/// (currently just the entry counter).
const META_TREE: &[u8] = b"__archiver_meta";

/// Key inside [`META_TREE`] that stores the cached entry counter as a u64-BE.
const META_KEY_COUNT: &[u8] = b"count";

/// Thread-safe handle to the root store. Cheap to clone (wraps Arc<sled::Db>).
#[derive(Clone)]
pub struct RootStore {
    db: Arc<sled::Db>,
    meta: sled::Tree,
    /// Cached entry count — avoids O(n) scan on every status request.
    entry_count: Arc<AtomicUsize>,
}

impl RootStore {
    /// Open (or create) the sled database at the given path.
    ///
    /// On open, the entry counter is seeded from a persisted side tree (O(1));
    /// only when the counter is missing (first run after upgrade, or a freshly
    /// created database) do we fall back to a one-time `db.len()` scan and
    /// persist the result. This avoids re-warming sled's page cache with the
    /// full history on every restart.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::Config::default()
            .path(path.as_ref())
            .cache_capacity(SLED_CACHE_CAPACITY_BYTES)
            .open()
            .with_context(|| format!("failed to open sled database at {:?}", path.as_ref()))?;

        let meta = db
            .open_tree(META_TREE)
            .context("failed to open archiver meta tree")?;

        // The default tree (where roots live) is `db` itself, since `Db: Deref<Target=Tree>`.
        let initial_count = match meta.get(META_KEY_COUNT)? {
            Some(bytes) => {
                let arr: [u8; 8] = bytes
                    .as_ref()
                    .try_into()
                    .context("meta count value has wrong length (expected 8 bytes)")?;
                u64::from_be_bytes(arr) as usize
            }
            None => {
                // First run after upgrade, or fresh database. Pay the O(n) cost
                // exactly once and persist the result so subsequent restarts
                // don't have to repeat it.
                let scanned = db.len();
                meta.insert(META_KEY_COUNT, &(scanned as u64).to_be_bytes())?;
                scanned
            }
        };

        Ok(Self {
            db: Arc::new(db),
            meta,
            entry_count: Arc::new(AtomicUsize::new(initial_count)),
        })
    }

    /// Insert a batch of roots atomically.
    pub fn put_roots(&self, roots: &[(u64, H256)]) -> Result<()> {
        let mut batch = sled::Batch::default();
        for (height, root) in roots {
            batch.insert(&height.to_be_bytes(), root.as_bytes());
        }
        self.db
            .apply_batch(batch)
            .context("failed to apply batch insert")?;
        // Assumes inserts are unique (no overwrites). For backfill over existing
        // entries this may drift slightly, but that's acceptable for a status counter.
        let new_total = self
            .entry_count
            .fetch_add(roots.len(), Ordering::AcqRel)
            .saturating_add(roots.len());
        // Persist the running count to the meta tree. This is best-effort: it
        // does not need to be atomic with the roots batch — if we crash between
        // the two writes the counter will simply drift by at most one batch on
        // the next restart (and only until the next successful put_roots).
        // Failing to persist the counter must not abort archival.
        if let Err(e) = self
            .meta
            .insert(META_KEY_COUNT, &(new_total as u64).to_be_bytes())
        {
            tracing::warn!(error = %e, "failed to persist entry count to meta tree");
        }
        Ok(())
    }

    /// Get roots for an inclusive block range [from, to].
    /// Returns (block_number, merkle_root) pairs in ascending order.
    pub fn get_range(&self, from: u64, to: u64) -> Result<Vec<(u64, H256)>> {
        let start = from.to_be_bytes();
        let capacity = (to.saturating_sub(from) + 1) as usize;
        let mut results = Vec::with_capacity(capacity);

        for item in self.db.range(start..=to.to_be_bytes()) {
            let (key, value) = item.context("failed to read from sled")?;
            let height = parse_height(&key)?;
            let root = parse_h256(&value)?;
            results.push((height, root));
        }

        Ok(results)
    }

    /// Get the latest (highest) stored block height, or None if empty.
    pub fn latest_height(&self) -> Result<Option<u64>> {
        match self.db.last()? {
            Some((key, _)) => Ok(Some(parse_height(&key)?)),
            None => Ok(None),
        }
    }

    /// Find gaps in the stored block range.
    /// Returns a list of `(start, end)` inclusive ranges that are missing.
    pub fn find_gaps(&self) -> Result<Vec<(u64, u64)>> {
        let mut gaps = Vec::new();
        let mut expected: Option<u64> = None;

        for item in self.db.iter() {
            let (key, _) = item.context("failed to read from sled")?;
            let height = parse_height(&key)?;

            match expected {
                Some(exp) if height > exp => gaps.push((exp, height - 1)),
                _ => {}
            }
            expected = Some(height + 1);
        }

        Ok(gaps)
    }

    /// Return the cached entry count (O(1), updated on each `put_roots` call).
    pub fn count(&self) -> usize {
        self.entry_count.load(Ordering::Acquire)
    }

    /// Flush database to disk.
    pub async fn flush(&self) -> Result<()> {
        self.db.flush_async().await?;
        Ok(())
    }
}

fn parse_height(key: &sled::IVec) -> Result<u64> {
    let bytes: [u8; 8] = key
        .as_ref()
        .try_into()
        .with_context(|| format!("invalid key length: expected 8, got {}", key.len()))?;
    Ok(u64::from_be_bytes(bytes))
}

fn parse_h256(value: &sled::IVec) -> Result<H256> {
    anyhow::ensure!(
        value.len() == 32,
        "invalid digest length: expected 32, got {}",
        value.len()
    );
    Ok(H256::from_slice(value.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_put_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let root = H256::random();
        store.put_roots(&[(42, root)]).unwrap();

        let range = store.get_range(42, 42).unwrap();
        assert_eq!(range.len(), 1);
        assert_eq!(range[0], (42, root));
        assert_eq!(store.get_range(43, 43).unwrap().len(), 0);
        assert_eq!(store.latest_height().unwrap(), Some(42));
    }

    #[test]
    fn range_query() {
        let dir = tempfile::tempdir().unwrap();
        let store = RootStore::open(dir.path().join("test.sled")).unwrap();

        let roots: Vec<H256> = (0..10).map(|_| H256::random()).collect();
        let entries: Vec<(u64, H256)> = roots
            .iter()
            .enumerate()
            .map(|(i, r)| (i as u64, *r))
            .collect();
        store.put_roots(&entries).unwrap();

        let range = store.get_range(3, 6).unwrap();
        assert_eq!(range.len(), 4);
        assert_eq!(range[0], (3, roots[3]));
        assert_eq!(range[3], (6, roots[6]));
    }

    #[test]
    fn count_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sled");

        {
            let store = RootStore::open(&path).unwrap();
            let entries: Vec<(u64, H256)> = (0..7).map(|i| (i, H256::random())).collect();
            store.put_roots(&entries).unwrap();
            assert_eq!(store.count(), 7);
            // Drop the store, closing the database.
        }

        let reopened = RootStore::open(&path).unwrap();
        // Count must be restored from the meta tree without a full scan.
        assert_eq!(reopened.count(), 7);

        reopened.put_roots(&[(7, H256::random())]).unwrap();
        assert_eq!(reopened.count(), 8);
    }

    #[test]
    fn count_recovers_from_missing_meta() {
        // Simulate the upgrade path: a database that pre-exists without the
        // meta tree entry. The first open should one-time-scan and persist.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sled");

        {
            // Open with the new code, write some entries, then manually wipe
            // the meta key to mimic a pre-upgrade db.
            let store = RootStore::open(&path).unwrap();
            let entries: Vec<(u64, H256)> = (0..5).map(|i| (i, H256::random())).collect();
            store.put_roots(&entries).unwrap();
            store.meta.remove(META_KEY_COUNT).unwrap();
            store.db.flush().unwrap();
        }

        let reopened = RootStore::open(&path).unwrap();
        // The one-time fallback scan should have recovered the count.
        assert_eq!(reopened.count(), 5);
        // And persisted it for next time.
        assert!(reopened.meta.get(META_KEY_COUNT).unwrap().is_some());
    }
}
